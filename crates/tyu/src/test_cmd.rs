//! `tyu test` subcommand — suite runner that builds, executes, and verifies
//! test images across one or many targets.

use std::collections::HashSet;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use codegen_core::{FeatureSet, Target};

use harness_core::{
    AxisGate, CoverageAxis, QemuAxisGate, QualificationReport, SelectionReport, SkipReason,
    SkippedFixture, Verdict, REPORT_SCHEMA_VERSION,
};

use crate::args::{ReportFormat, TestArgs};
use crate::build;
use crate::elf_reader;
use crate::highwater::check_stack_witness;
use crate::manifest::{
    parse_manifest, validate_manifest_integrity, FixtureEntry, PoisonExpectation,
};
use crate::platform;
use crate::platform::ResolvedPlatformSelection;
use crate::runner::Runner;
use crate::test_helpers::workspace_root;

/// All known targets for `--all-targets`.
const ALL_TARGETS: &[Target] = &[
    Target::X86_64UnknownLinuxGnu,
    Target::X86_64UnknownNone,
    Target::ArmV7MUnknownNone,
    Target::RiscV32UnknownNone,
];

/// Tool names required per target.
fn required_tools(target: Target) -> &'static [&'static str] {
    match target {
        Target::X86_64UnknownLinuxGnu => &["langc"],
        Target::X86_64UnknownNone => &["langc", "fasm", "ld", "qemu-system-x86_64"],
        Target::ArmV7MUnknownNone => &[
            "langc",
            "arm-none-eabi-as",
            "arm-none-eabi-ld",
            "qemu-system-arm",
        ],
        Target::RiscV32UnknownNone => &[
            "langc",
            "riscv32-elf-as",
            "riscv32-elf-ld",
            "qemu-system-riscv32",
        ],
    }
}

#[derive(Clone, Debug)]
enum TestSelection {
    Target(Target),
    Platform(ResolvedPlatformSelection),
}

impl TestSelection {
    fn target(&self) -> Target {
        match self {
            Self::Target(target) => *target,
            Self::Platform(selection) => selection.target,
        }
    }

    fn platform_selection(&self) -> Option<&ResolvedPlatformSelection> {
        match self {
            Self::Target(_) => None,
            Self::Platform(selection) => Some(selection),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Target(target) => std::str::from_utf8(target.triple())
                .unwrap_or("<invalid>")
                .to_string(),
            Self::Platform(selection) => format!(
                "{}:{}",
                selection.pack.name(),
                std::str::from_utf8(selection.target.triple()).unwrap_or("<invalid>")
            ),
        }
    }

    fn capabilities(&self) -> HashSet<String> {
        match self {
            Self::Target(target) => platform::capabilities_for_target(*target),
            Self::Platform(selection) => platform::capabilities_for_selection(selection),
        }
    }
}

/// Run the `test` subcommand.
pub fn run(args: &TestArgs) -> Result<(), String> {
    // `--mode=dynamic` is not yet supported by this suite runner. The harness
    // links a *separate* generated `TestRunner` module against each fixture
    // (fixture-as-lib + runner-with-`main`), but the dynamic load path embeds a
    // single application module per modpack (v1 modpack carries exactly one
    // module, with one `.lang.modinfo`). Packing two modules would require a
    // multi-module modpack + boot-loop, or regenerating each fixture as a
    // self-contained module. Until then, fail loudly rather than silently
    // running static — dynamic loading is already exercised per-target by the
    // cargo suites: execution-tests `dynamic_{signed,negative,encrypted}` (x86)
    // and `arm`/`riscv` `dynamic_lmod_runs_under_qemu`.
    if matches!(args.mode, Some(crate::args::BuildMode::Dynamic)) {
        return Err(
            "tyu test --mode=dynamic is not yet supported (the fixture+runner harness \
             is two-module; modpack v1 loads a single module). Dynamic loading is \
             covered by the cargo suites: `cargo test -p execution-tests --test \
             dynamic_signed --test dynamic_negative --test dynamic_encrypted` and the \
             `arm`/`riscv` `dynamic_lmod_runs_under_qemu` tests."
                .to_string(),
        );
    }

    // Read manifest.
    let manifest = parse_manifest(&args.manifest_path)?;
    let fixtures_dir = args
        .manifest_path
        .parent()
        .ok_or("manifest has no parent directory")?;
    validate_manifest_integrity(&manifest, fixtures_dir)?;

    // Filter by name if --filter given.
    let filtered: Vec<&FixtureEntry> = manifest
        .fixtures
        .iter()
        .filter(|f| {
            if let Some(ref pat) = args.filter {
                f.name.contains(pat.as_str())
            } else {
                true
            }
        })
        .collect();

    if filtered.is_empty() {
        eprintln!("tyu: no matching fixtures");
        return Ok(());
    }

    // --all-platforms runs each QEMU-capable pack in its OWN `tyu test
    // --platform <name>` subprocess. Platforms are treated separately so no
    // runner/QEMU lifecycle state is ever shared across platforms within a
    // single process; each child is exactly the proven standalone path.
    if args.all_platforms {
        return run_all_platforms_isolated(args);
    }

    let selections = resolve_test_selections(args)?;
    let mut any_failure = false;
    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let feature_set = args.feature_set;
    let mut selection_reports = Vec::new();
    let stdout_redirect = if args.format == ReportFormat::Json && args.report_out.is_none() {
        Some(StdoutRedirect::to_null()?)
    } else {
        None
    };

    for selection in &selections {
        let target = selection.target();
        let triple = std::str::from_utf8(target.triple()).unwrap();
        let target_caps = selection.capabilities();
        let mut acc = SelectionAccumulator::new(selection, &target_caps);

        // Check tool availability.
        let tools = required_tools(target);
        let missing: Vec<&str> = tools
            .iter()
            .filter(|t| !tool_available(t))
            .copied()
            .collect();
        if !missing.is_empty() {
            if std::env::var("CI").is_ok() {
                return Err(format!(
                    "tyu: target {} — missing required tools under CI: {}. \
                     Install them or add them to PATH.",
                    triple,
                    missing.join(", "),
                ));
            }
            eprintln!(
                "tyu: target {} skipped — missing tools: {}",
                triple,
                missing.join(", "),
            );
            acc.reasons
                .push(format!("missing-tools: {}", missing.join(",")));
            selection_reports.push(acc.finish());
            continue;
        }

        // Filter fixtures by QEMU machine capability and platform capability.
        let mut eligible: Vec<&FixtureEntry> = Vec::new();
        for fixture in &filtered {
            if let Some(detail) = target_skip_detail(fixture, target) {
                acc.skipped.push(SkippedFixture {
                    name: fixture.name.clone(),
                    reason: SkipReason::TargetUnsupported,
                    detail,
                });
                continue;
            }
            // QemuSpec axis gating: mmio/interrupt fixtures require the
            // target's QEMU machine to advertise the corresponding feature.
            if let Some(detail) = fixture_qemu_eligible(fixture, target) {
                acc.skipped.push(SkippedFixture {
                    name: fixture.name.clone(),
                    reason: SkipReason::QemuUnsupported,
                    detail,
                });
                continue;
            }
            let missing_requirements: Vec<&str> = fixture
                .requires
                .iter()
                .map(String::as_str)
                .filter(|r| !target_caps.contains(*r))
                .collect();
            if missing_requirements.is_empty() {
                eligible.push(fixture);
            } else {
                acc.skipped.push(SkippedFixture {
                    name: fixture.name.clone(),
                    reason: SkipReason::CapabilityMissing,
                    detail: format!("missing: {}", missing_requirements.join(",")),
                });
            }
        }

        if eligible.is_empty() {
            acc.reasons.push("no-eligible-fixtures".to_string());
            selection_reports.push(acc.finish());
            continue;
        }

        // Build and run each suite.
        for fixture in eligible {
            let fixture_path = fixtures_dir.join(&fixture.file);
            if !fixture_path.exists() {
                acc.skipped.push(SkippedFixture {
                    name: fixture.name.clone(),
                    reason: SkipReason::FileMissing,
                    detail: fixture_path.display().to_string(),
                });
                acc.reasons.push(format!(
                    "fixture '{}' not found at '{}'",
                    fixture.name,
                    fixture_path.display()
                ));
                any_failure = true;
                total_failed += 1;
                acc.any_fixture_failed = true;
                continue;
            }

            let result = run_single_suite(fixture, &selection, fixtures_dir, feature_set);
            let result = poison_verdict(fixture, result);
            match result {
                Ok(()) => {
                    acc.ran.push(fixture.name.clone());
                    acc.covered.extend(fixture.axes.iter().copied());
                    total_passed += 1;
                }
                Err(e) => {
                    acc.ran.push(fixture.name.clone());
                    acc.reasons
                        .push(format!("fixture={} failed: {}", fixture.name, e));
                    any_failure = true;
                    total_failed += 1;
                    acc.any_fixture_failed = true;
                }
            }
        }
        selection_reports.push(acc.finish());
    }

    let report = QualificationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        selections: selection_reports,
    };
    drop(stdout_redirect);
    if let Some(path) = args.report_out.as_deref() {
        write_report(path, &report)?;
    } else {
        render_report(&report, args.format)?;
    }

    // Machine-parseable one-line summary on stdout (skipped for JSON-to-stdout,
    // where stdout carries the report document). CI parses the fixture count
    // from this line to guard against a vacuous pass (zero fixtures run).
    if !(args.format == ReportFormat::Json && args.report_out.is_none()) {
        let status = if any_failure { "FAILED" } else { "ok" };
        println!("test result: {status}. {total_passed} passed; {total_failed} failed",);
    }

    if any_failure || (args.qualify && report_has_failure(&report)) {
        Err("some tests failed".into())
    } else {
        Ok(())
    }
}

struct StdoutRedirect {
    saved_fd: i32,
}

impl StdoutRedirect {
    fn to_null() -> Result<Self, String> {
        let dev_null = File::options()
            .write(true)
            .open("/dev/null")
            .map_err(|e| format!("opening /dev/null for json report isolation: {}", e))?;
        let saved_fd = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved_fd < 0 {
            return Err(format!(
                "duplicating stdout for json report isolation: {}",
                std::io::Error::last_os_error()
            ));
        }
        let rc = unsafe { libc::dup2(dev_null.as_raw_fd(), libc::STDOUT_FILENO) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::close(saved_fd);
            }
            return Err(format!(
                "redirecting stdout for json report isolation: {}",
                err
            ));
        }
        Ok(Self { saved_fd })
    }
}

impl Drop for StdoutRedirect {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.saved_fd, libc::STDOUT_FILENO);
            libc::close(self.saved_fd);
        }
    }
}

struct SelectionAccumulator {
    label: String,
    target: String,
    rung_proven: String,
    advertised_capabilities: Vec<String>,
    ran: Vec<String>,
    skipped: Vec<SkippedFixture>,
    required_axes: Vec<CoverageAxis>,
    covered: HashSet<CoverageAxis>,
    any_fixture_failed: bool,
    reasons: Vec<String>,
}

impl SelectionAccumulator {
    fn new(selection: &TestSelection, capabilities: &HashSet<String>) -> Self {
        let target = selection.target();
        let mut advertised_capabilities: Vec<String> = capabilities.iter().cloned().collect();
        advertised_capabilities.sort();
        Self {
            label: selection.label(),
            target: std::str::from_utf8(target.triple())
                .unwrap_or("<invalid>")
                .to_string(),
            rung_proven: selection_rung(selection),
            advertised_capabilities,
            ran: Vec::new(),
            skipped: Vec::new(),
            required_axes: required_axes_for_selection(selection, capabilities),
            covered: HashSet::new(),
            any_fixture_failed: false,
            reasons: Vec::new(),
        }
    }

    fn finish(mut self) -> SelectionReport {
        self.ran.sort();
        self.skipped.sort_by(|a, b| a.name.cmp(&b.name));
        let covered_axes = canonical_axes_from_set(&self.covered);
        let covered_set: HashSet<CoverageAxis> = covered_axes.iter().copied().collect();
        let uncovered_axes = self
            .required_axes
            .iter()
            .copied()
            .filter(|axis| !covered_set.contains(axis))
            .collect::<Vec<_>>();
        let verdict = if self.any_fixture_failed || !uncovered_axes.is_empty() {
            Verdict::Fail
        } else {
            Verdict::Pass
        };
        SelectionReport {
            label: self.label,
            target: self.target,
            rung_proven: self.rung_proven,
            advertised_capabilities: self.advertised_capabilities,
            ran: self.ran,
            skipped: self.skipped,
            required_axes: self.required_axes,
            covered_axes,
            uncovered_axes,
            verdict,
            reasons: self.reasons,
        }
    }
}

fn selection_rung(selection: &TestSelection) -> String {
    match selection {
        TestSelection::Target(_) => "target".to_string(),
        TestSelection::Platform(selection) => {
            selection.pack.manifest.test.rung.as_str().to_string()
        }
    }
}

fn target_triple_string(target: Target) -> String {
    std::str::from_utf8(target.triple())
        .unwrap_or("<invalid>")
        .to_string()
}

fn target_skip_detail(fixture: &FixtureEntry, target: Target) -> Option<String> {
    if fixture.targets.is_empty() {
        return None;
    }
    let triple = target_triple_string(target);
    if fixture.targets.iter().any(|allowed| allowed == &triple) {
        None
    } else {
        Some(format!(
            "target {} not in [{}]",
            triple,
            fixture.targets.join(",")
        ))
    }
}

/// Check whether a fixture's Qemu-gated axes are supported by the target's
/// QEMU machine.  Returns `Some(detail)` if the fixture should be skipped;
/// `None` if it is eligible for execution or has no Qemu-gated axes.
fn fixture_qemu_eligible(fixture: &FixtureEntry, target: Target) -> Option<String> {
    let qemu = match target.spec().qemu {
        Some(q) => q,
        None => {
            // No QEMU at all → any Qemu-gated axis is ineligible.
            if fixture
                .axes
                .iter()
                .any(|a| matches!(a.gate(), AxisGate::Qemu(_)))
            {
                return Some("target has no QEMU support".into());
            }
            return None;
        }
    };
    for axis in &fixture.axes {
        match axis.gate() {
            AxisGate::Qemu(QemuAxisGate::MmioScratch) => {
                if qemu.mmio_scratch.is_none() {
                    return Some(format!(
                        "axis '{}' requires mmio_scratch which this target lacks",
                        axis.as_str()
                    ));
                }
            }
            AxisGate::Qemu(QemuAxisGate::InterruptSource) => {
                if qemu.interrupt_source.is_none() {
                    return Some(format!(
                        "axis '{}' requires interrupt_source which this target lacks",
                        axis.as_str()
                    ));
                }
            }
            _ => {}
        }
    }
    None
}

fn required_axes_for_selection(
    selection: &TestSelection,
    capabilities: &HashSet<String>,
) -> Vec<CoverageAxis> {
    let target = selection.target();
    let mut required = HashSet::new();
    for axis in CoverageAxis::ALL {
        let is_required = match axis.gate() {
            AxisGate::Core => target.spec().qemu.is_some(),
            AxisGate::RuntimeService(services) => services
                .iter()
                .any(|service| capabilities.contains(*service)),
            AxisGate::Qemu(qag) => target.spec().qemu.map_or(false, |q| match qag {
                QemuAxisGate::MmioScratch => q.mmio_scratch.is_some(),
                QemuAxisGate::InterruptSource => q.interrupt_source.is_some(),
            }),
        };
        if is_required {
            required.insert(axis);
        }
    }
    canonical_axes_from_set(&required)
}

fn canonical_axes_from_set(set: &HashSet<CoverageAxis>) -> Vec<CoverageAxis> {
    CoverageAxis::ALL
        .iter()
        .copied()
        .filter(|axis| set.contains(axis))
        .collect()
}

fn render_report(report: &QualificationReport, format: ReportFormat) -> Result<(), String> {
    match format {
        ReportFormat::Human => {
            eprint!("{}", harness_core::render_human(report));
            Ok(())
        }
        ReportFormat::Json => {
            let json = serde_json::to_string_pretty(report)
                .map_err(|e| format!("serializing test report: {}", e))?;
            println!("{json}");
            Ok(())
        }
    }
}

fn write_report(path: &Path, report: &QualificationReport) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating report dir '{}': {}", parent.display(), e))?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|e| format!("serializing test report '{}': {}", path.display(), e))?;
    std::fs::write(path, json).map_err(|e| format!("writing report '{}': {}", path.display(), e))
}

fn report_has_failure(report: &QualificationReport) -> bool {
    report
        .selections
        .iter()
        .any(|selection| selection.verdict == Verdict::Fail)
}

fn resolve_test_selections(args: &TestArgs) -> Result<Vec<TestSelection>, String> {
    if let Some(name) = args.platform.as_deref() {
        let selection =
            platform::resolve_platform_selection(&workspace_root(), name, args.isa.as_deref())?;
        return Ok(vec![TestSelection::Platform(selection)]);
    }

    if args.all_targets {
        return Ok(ALL_TARGETS
            .iter()
            .copied()
            .map(TestSelection::Target)
            .collect());
    }

    Ok(vec![TestSelection::Target(args.target)])
}

/// Run each QEMU-capable platform pack in an isolated `tyu test --platform`
/// subprocess. Platforms are treated separately: no in-process runner or QEMU
/// lifecycle state crosses platform boundaries, which keeps `--all-platforms`
/// behaviorally identical to running each `tyu test --platform <name>` by hand.
///
/// Hardware-rung packs (e.g. RP2350) are **not** run in emulation.  They
/// appear in the aggregate report as explicit `verdict=n/a` entries so the
/// emulator-coverage gap is surfaced, never silently dropped (FR-9).
fn run_all_platforms_isolated(args: &TestArgs) -> Result<(), String> {
    let root = workspace_root();
    let exe = std::env::current_exe()
        .map_err(|e| format!("locating tyu executable for isolated platform run: {}", e))?;

    let mut any_failure = false;
    let mut aggregate = QualificationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        selections: Vec::new(),
    };

    // Phase 1: discover all platform packs.  Hardware-rung packs get an
    // honest NotApplicable entry; QEMU-capable packs proceed to Phase 2.
    let all_packs = platform::discover_platforms_in(&root)?;
    let mut qemu_selections: Vec<TestSelection> = Vec::new();
    for pack in &all_packs {
        if pack.manifest.test.rung == platform::TestRung::Hardware {
            aggregate.selections.push(hardware_report_entry(pack));
            continue;
        }
        let selection = match platform::resolve_platform_selection(&root, pack.name(), None) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !platform::is_qemu_capable_selection(&selection) {
            continue;
        }
        qemu_selections.push(TestSelection::Platform(selection));
    }

    if qemu_selections.is_empty() && aggregate.selections.is_empty() {
        eprintln!("tyu: no platform packs to test");
        return Ok(());
    }

    // Phase 2: process each QEMU-capable pack in an isolated child process.
    for (index, selection) in qemu_selections.iter().enumerate() {
        let name = match selection.platform_selection() {
            Some(sel) => sel.pack.name().to_string(),
            None => continue,
        };
        let report_path = child_report_path(&name, index);

        let mut cmd = Command::new(&exe);
        cmd.arg("test")
            .arg(format!("--platform={}", name))
            .arg(format!("--manifest={}", args.manifest_path.display()))
            .arg(format!("--report-out={}", report_path.display()));
        if let Some(ref filter) = args.filter {
            cmd.arg(format!("--filter={}", filter));
        }
        if let Some(ref profile) = args.profile {
            cmd.arg(format!("--profile={}", profile));
        } else {
            cmd.arg(format!("--features={}", feature_set_arg(args.feature_set)));
        }
        if args.qualify {
            cmd.arg("--qualify");
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::null());

        let status = cmd
            .status()
            .map_err(|e| format!("running isolated platform test for {}: {}", name, e))?;
        match read_child_report(selection, &report_path) {
            Ok(mut report) => {
                let child_failed =
                    !status.success() || (args.qualify && report_has_failure(&report));
                any_failure |= child_failed;
                aggregate.selections.append(&mut report.selections);
            }
            Err(reason) => {
                any_failure = true;
                aggregate
                    .selections
                    .push(synthetic_fail_selection(selection, reason));
            }
        }
        let _ = std::fs::remove_file(&report_path);
        if !status.success() {
            any_failure = true;
        }
    }

    if let Some(path) = args.report_out.as_deref() {
        write_report(path, &aggregate)?;
    } else {
        render_report(&aggregate, args.format)?;
    }

    if any_failure || (args.qualify && report_has_failure(&aggregate)) {
        Err("some tests failed".into())
    } else {
        Ok(())
    }
}

/// Build a `SelectionReport` for a hardware-rung pack that is excluded from
/// emulator qualification.  The report carries `verdict=n/a` and a reason
/// explaining that the platform requires hardware-in-the-loop testing.
fn hardware_report_entry(pack: &platform::PlatformPack) -> SelectionReport {
    let default_isa = pack
        .manifest
        .platform
        .isa
        .iter()
        .find(|isa| isa.default)
        .or_else(|| pack.manifest.platform.isa.first());
    let target_triple = default_isa
        .map(|isa| isa.triple.clone())
        .unwrap_or_default();
    SelectionReport {
        label: format!("hardware:{}", pack.name()),
        target: target_triple,
        rung_proven: "hardware".to_string(),
        advertised_capabilities: Vec::new(),
        ran: Vec::new(),
        skipped: Vec::new(),
        required_axes: Vec::new(),
        covered_axes: Vec::new(),
        uncovered_axes: Vec::new(),
        verdict: Verdict::NotApplicable,
        reasons: vec!["hardware-only: not emulator-tested".to_string()],
    }
}

fn child_report_path(platform_name: &str, index: usize) -> PathBuf {
    let sanitized: String = platform_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    std::env::temp_dir().join("tyu_report").join(format!(
        "{}.{}.{}.json",
        std::process::id(),
        index,
        sanitized
    ))
}

fn feature_set_arg(feature_set: FeatureSet) -> String {
    let mut buf = [""; 8];
    let n = feature_set.write_flags(&mut buf);
    buf[..n].join(",")
}

fn read_child_report(
    selection: &TestSelection,
    path: &Path,
) -> Result<QualificationReport, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("missing child report '{}': {}", path.display(), e))?;
    let report: QualificationReport = serde_json::from_str(&text)
        .map_err(|e| format!("malformed child report '{}': {}", path.display(), e))?;
    if report.schema_version != REPORT_SCHEMA_VERSION {
        return Err(format!(
            "schema-version-mismatch: expected {} got {}",
            REPORT_SCHEMA_VERSION, report.schema_version
        ));
    }
    if report.selections.is_empty() {
        return Err(format!(
            "empty child report for selection {}",
            selection.label()
        ));
    }
    Ok(report)
}

fn synthetic_fail_selection(selection: &TestSelection, reason: String) -> SelectionReport {
    let capabilities = selection.capabilities();
    let mut acc = SelectionAccumulator::new(selection, &capabilities);
    acc.any_fixture_failed = true;
    acc.reasons.push(reason);
    acc.finish()
}

/// Build and run a single test suite (a set of fixtures).
fn run_single_suite(
    fixture: &FixtureEntry,
    selection: &TestSelection,
    fixtures_dir: &Path,
    feature_set: FeatureSet,
) -> Result<(), String> {
    let target = selection.target();
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let out_dir = std::env::temp_dir().join("tyu_test").join(format!(
        "{}_{}_{}",
        triple,
        fixture.name,
        std::process::id()
    ));
    let build_ctx = build::BuildContext {
        target,
        out_dir: out_dir.clone(),
        platform_selection: selection.platform_selection().cloned(),
    };

    // Build langc first.
    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    // Generate test runner.
    let runner_src = generate_runner(&[fixture], target);
    let runner_path = out_dir.join("test_runner.mod");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("creating out_dir: {}", e))?;
    std::fs::write(&runner_path, &runner_src).map_err(|e| format!("writing test_runner: {}", e))?;

    // Write a .def file for the fixture so the generated runner can import it.
    // langc does not emit .def files, so we write one from the fixture metadata.
    let mod_name = fixture_module_name(&fixture.name);
    let run_word = fixture_run_word(&fixture.name);
    let def_content =
        format!("module {mod_name};\nexport {{ {run_word} }};\n: {run_word} ( -- ) ;\nend;\n");
    let def_path = out_dir.join(format!("{}.def", mod_name));
    let _ = std::fs::write(&def_path, &def_content);

    // Compile the fixture as lib.
    let mut objs: Vec<PathBuf> = Vec::new();

    let fixture_path = fixtures_dir.join(&fixture.file);
    let fixture_o = compile_mod(&build_ctx, &fixture_path, true, feature_set)?;
    objs.push(fixture_o.clone());

    // Compile the runner.
    let runner_o = compile_mod(&build_ctx, &runner_path, false, feature_set)?;
    objs.push(runner_o);

    // Assemble runtime units.
    let runtime_objs = build::assemble_runtime_for_context(&build_ctx, feature_set)?;
    objs.extend(runtime_objs);

    // Link.
    let image = build::link_image_for_context(&build_ctx, &objs)?;

    // Determine runner.
    let runner = Runner::for_target(target);

    // Run with timeout.
    let timeout = Duration::from_secs(10);
    let outcome = runner.run(&image, timeout)?;

    if outcome.timed_out {
        let classify = crate::debug_escalate::classify_hang(&image, target);
        return Err(format!(
            "HANG (timed out after {:?})\n  {}",
            timeout, classify,
        ));
    }

    // Parse output.
    let summary = harness_core::parse_output(&outcome.stdout);

    // Try to decode any D diagnostic records from the output.
    let diag_text = decode_diags_from_stdout(&outcome.stdout, &fixture_o, Some(&fixture_path));

    // Build source map for escalation (if needed).
    let mut source_map = diag_core::render::SourceMap::new();
    source_map.add_file(&fixture_path);

    if !summary.completed {
        let exit = outcome.exit_code;

        // If no D records, try A-side escalation.
        let escalate_text = if diag_text.is_empty() && target.spec().qemu.is_some() {
            let esc =
                crate::debug_escalate::escalate(&image, target, Some((&fixture_path, &source_map)));
            let crate::debug_escalate::EscalationOutcome {
                diagnostic_string,
                error,
                target: esc_target,
                mode,
                port,
                phase,
            } = esc;
            match (diagnostic_string, error) {
                (Some(diag), None) => Some(diag),
                (Some(diag), Some(err)) => {
                    return Err(format!(
                        "escalation produced diagnostic but also reported error (target={:?} mode={:?} port={} phase={:?}): {}\n{}",
                        esc_target, mode, port, phase, err, diag,
                    ));
                }
                (None, Some(err)) => {
                    return Err(format!(
                        "escalation failed (target={:?} mode={:?} port={} phase={:?}): {}",
                        esc_target, mode, port, phase, err,
                    ));
                }
                (None, None) => {
                    return Err(format!(
                        "escalation returned no diagnostic and no error (target={:?} mode={:?} port={} phase={:?})",
                        esc_target, mode, port, phase,
                    ));
                }
            }
        } else {
            None
        };

        let extra = escalate_text
            .or_else(|| {
                if diag_text.is_empty() {
                    None
                } else {
                    Some(diag_text)
                }
            })
            .map(|t| format!("\n{}", t))
            .unwrap_or_default();

        return Err(format!(
            "NO_COMPLETION — exited with code {} but no `S\\n` marker in output{}",
            exit, extra,
        ));
    }

    if summary.failures > 0 {
        let extra = if diag_text.is_empty() {
            String::new()
        } else {
            format!("\n{}", diag_text)
        };
        return Err(format!(
            "FAIL_MARKER — {} failure(s) reported via 'F' bytes{}",
            summary.failures, extra,
        ));
    }

    // Check QEMU/native exit code.
    let expected = match target.spec().qemu {
        Some(spec) => spec.exit_convention.host_pass_exit(),
        None => 0,
    };
    if outcome.exit_code != expected {
        return Err(format!(
            "EXIT_MISMATCH — exit code {} != expected {}",
            outcome.exit_code, expected,
        ));
    }

    // Assertion-count check.
    if let Some(expected) = fixture.expects {
        if summary.assertions == 0 {
            return Err(format!(
                "NO_ASSERTIONS: fixture '{}' declares expects={} but zero assertions were executed. \
                 The `P` record counter mechanism may not be wired.",
                fixture.name, expected,
            ));
        }
        if summary.assertions < expected {
            return Err(format!(
                "UNDERRAN: fixture '{}' declares expects={} but only {} assertions executed",
                fixture.name, expected, summary.assertions,
            ));
        }
        if summary.assertions > expected {
            // A fixture running extra assertions is a test-integrity concern.
            // We warn rather than fail to avoid brittleness.
            eprintln!(
                "tyu: fixture '{}' ran {} assertions (expects={})",
                fixture.name, summary.assertions, expected,
            );
        }
    }

    // Stack-bound witness check (generalized H/D witness).
    check_stack_witness(summary.high_slots, summary.diagnostics > 0, &image)?;

    Ok(())
}

/// Parse D records from serial output and attempt to decode them against
/// the fixture's `.lang.modinfo`.  Returns a formatted diagnostic string,
/// or empty if no D records are found or modinfo is unavailable.
///
/// `fixture_o` is the path to the compiled object file (for ELF section
/// reading).  `fixture_src` is the path to the source `.mod` file (for
/// source-context rendering); pass `None` to skip source context.
fn decode_diags_from_stdout(stdout: &[u8], fixture_o: &Path, fixture_src: Option<&Path>) -> String {
    let records: Vec<harness_core::Record<'_>> = harness_core::parse_records(stdout).collect();

    let d_records: Vec<&[u8]> = records
        .iter()
        .filter_map(|r| {
            if let harness_core::Record::Diag(payload) = r {
                Some(*payload)
            } else {
                None
            }
        })
        .collect();

    if d_records.is_empty() {
        return String::new();
    }

    // Try .lang.debug first (full coverage), fall back to .lang.modinfo.
    let elf_bytes = match std::fs::read(fixture_o) {
        Ok(b) => b,
        Err(_) => {
            return format!(
                "[{} D record(s) present but cannot read fixture]",
                d_records.len()
            )
        }
    };

    // Read both sections from the ELF.
    let debug_bytes = elf_reader::read_elf_section(&elf_bytes, b".lang.debug");
    let modinfo_bytes = elf_reader::read_elf_section(&elf_bytes, b".lang.modinfo");

    let index = if let Some(ref dbg) = debug_bytes {
        match diag_core::decode::ModinfoIndex::from_debug_bytes(dbg) {
            Some(idx) => idx,
            None => {
                return format!(
                    "[{} D record(s) present but .lang.debug is malformed]",
                    d_records.len()
                )
            }
        }
    } else if let Some(ref minfo) = modinfo_bytes {
        match diag_core::decode::ModinfoIndex::from_modinfo_bytes(minfo) {
            Some(idx) => idx,
            None => {
                return format!(
                    "[{} D record(s) present but .lang.modinfo is malformed]",
                    d_records.len()
                )
            }
        }
    } else {
        return format!(
            "[{} D record(s) present but no .lang.debug or .lang.modinfo in fixture]",
            d_records.len()
        );
    };

    // Load the fixture source into a SourceMap for context.
    let mut source_map = diag_core::render::SourceMap::new();
    if let Some(src_path) = fixture_src {
        source_map.add_file(src_path);
    }

    let mut lines: Vec<String> = Vec::new();
    for payload in &d_records {
        match diag_core::DiagRecord::parse(payload) {
            Some(record) => {
                let diag = diag_core::decode::resolve(&record, &index);
                let rendered = diag.render(fixture_src, &source_map);
                lines.push(format!("  D: {}", rendered));
            }
            None => lines.push("  D: <malformed DiagRecord>".into()),
        }
    }
    lines.join("\n")
}

/// Adjust a suite result for poison fixtures.
///
/// For a normal (non-poison) fixture the result passes through unchanged.
/// For a poison fixture the verdict is inverted:
/// - Expected failure → pass (`Ok(())`).
/// - Clean run or unexpected failure → `Err("POISON_DID_NOT_FAIL")`.
fn poison_verdict(fixture: &FixtureEntry, run_result: Result<(), String>) -> Result<(), String> {
    let poison = match fixture.poison {
        Some(ref p) => p,
        None => return run_result,
    };

    match run_result {
        Ok(()) => Err(
            "POISON_DID_NOT_FAIL — poison fixture completed without the expected failure".into(),
        ),
        Err(ref msg) => {
            let poison_occurred = match poison {
                PoisonExpectation::FailMarker => msg.contains("FAIL_MARKER"),
                PoisonExpectation::NoCompletion => {
                    msg.contains("NO_COMPLETION") || msg.contains("HANG")
                }
                PoisonExpectation::Trap(code) => {
                    let no_comp_or_hang = msg.contains("NO_COMPLETION") || msg.contains("HANG");
                    if !no_comp_or_hang {
                        return Err(format!(
                            "POISON_DID_NOT_FAIL — expected trap:{} but got different failure: {}",
                            code, msg,
                        ));
                    }
                    if *code == 0 {
                        // Trap code 0 = accept any trap.
                        true
                    } else {
                        // x86_64 (isa-debug-exit): exit code carries trap code.
                        // The NO_COMPLETION message includes "code <N>".
                        let exit_code_str = format!("code {}", code);
                        // ARM/RISC-V (semihosting): D diagnostic records carry
                        // the trap code.  The rendered diag line includes "(<N>)"
                        // (e.g. "STACK_OVERFLOW (10)").  Match that pattern too.
                        let diag_code_str = format!("({})", code);
                        msg.contains(&exit_code_str) || msg.contains(&diag_code_str)
                    }
                }
            };
            if poison_occurred {
                Ok(())
            } else {
                Err(format!(
                    "POISON_DID_NOT_FAIL — expected poison outcome did not occur: {}",
                    msg,
                ))
            }
        }
    }
}

/// Generate a test_runner.mod that imports the given fixture and calls its
/// test-run word, then emits S\n and returns 0.
fn generate_runner(fixtures: &[&FixtureEntry], target: Target) -> String {
    let mut out = String::from("module TestRunner;\n");
    for f in fixtures {
        let mod_name = fixture_module_name(&f.name);
        let word = fixture_run_word(&f.name);
        out.push_str(&format!("import {mod_name} {{ {word} }};\n"));
    }
    if target == Target::X86_64UnknownLinuxGnu {
        out.push_str("import platform/linux { platform.io.log };\n");
    } else {
        out.push_str("import platform/testio { testio.write-byte };\n");
    }
    out.push('\n');
    out.push_str(": emit-done ( -- )\n");
    if target == Target::X86_64UnknownLinuxGnu {
        out.push_str("  \"S\\n\" platform.io.log ;\n");
    } else {
        out.push_str("  83 testio.write-byte\n"); // 'S'
        out.push_str("  10 testio.write-byte ;\n"); // '\n'
    }
    out.push('\n');
    out.push_str(": main ( -- i64 )\n");
    for f in fixtures {
        out.push_str(&format!("  {}\n", fixture_run_word(&f.name)));
    }
    out.push_str("  emit-done\n");
    out.push_str("  0 ;\n");
    out.push('\n');
    out.push_str("export { main };\n");
    out.push_str("end;\n");
    out
}

fn fixture_run_word(fixture: &str) -> String {
    format!("{}-run", fixture.replace('_', "-"))
}

fn fixture_module_name(fixture: &str) -> String {
    fixture
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}

/// Compile a .mod file with langc.
fn compile_mod(
    ctx: &build::BuildContext,
    src: &Path,
    is_lib: bool,
    feature_set: FeatureSet,
) -> Result<PathBuf, String> {
    let sysroot = workspace_root().join("sysroot");
    // Include both the standard fixtures dir AND the out_dir so that
    // the test runner can import fixtures compiled into the same output
    // directory (their .def files are produced there).
    let mut include_dirs = vec![fixtures_dir()];
    include_dirs.push(ctx.out_dir.clone());
    build::compile_module_for_context(ctx, src, is_lib, Some(&sysroot), &include_dirs, feature_set)
        .map_err(|e| e.to_string())
}

fn fixtures_dir() -> PathBuf {
    workspace_root()
        .join("crates")
        .join("execution-tests")
        .join("fixtures")
}

fn tool_available(name: &str) -> bool {
    // Use the toolchain resolver so multi-named toolchains (e.g. RISC-V exposed
    // as `riscv64-linux-gnu-*` rather than `riscv64-unknown-elf-*`) are detected
    // by any accepted candidate — otherwise the target is falsely "skipped".
    crate::toolchain::resolve_tool(name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use codegen_core::{AssemblerKind, OutputFormat};

    fn target_selection() -> TestSelection {
        TestSelection::Target(Target::X86_64UnknownNone)
    }

    fn selection_report(verdict: Verdict) -> SelectionReport {
        SelectionReport {
            label: "x86_64-unknown-none".to_string(),
            target: "x86_64-unknown-none".to_string(),
            rung_proven: "target".to_string(),
            advertised_capabilities: Vec::new(),
            ran: vec!["arithmetic".to_string()],
            skipped: Vec::new(),
            required_axes: vec![CoverageAxis::Arith],
            covered_axes: vec![CoverageAxis::Arith],
            uncovered_axes: Vec::new(),
            verdict,
            reasons: Vec::new(),
        }
    }

    fn report_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tyu_test_cmd_{name}_{}.json", std::process::id()))
    }

    fn write_pack(root: &Path, rel: &str, name: &str, triple: &str, rung: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                r#"
[platform]
name = "{name}"
compiler-interface = 1

[[platform.isa]]
triple = "{triple}"
arch = "x86_64"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "{rung}"
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn hardware_pack_creates_not_applicable_report_entry() {
        let pack = platform::PlatformPack {
            manifest_path: PathBuf::from("/nonexistent/platform.toml"),
            root: PathBuf::from("/nonexistent"),
            manifest: platform::PlatformManifest {
                platform: platform::PlatformSection {
                    name: "rp2350".to_string(),
                    compiler_interface: 1,
                    description: None,
                    isa: vec![platform::IsaEntry {
                        triple: "armv7m-unknown-none".to_string(),
                        arch: "arm".to_string(),
                        default: true,
                        expected_abi_hash: None,
                        metal: None,
                    }],
                },
                metal: platform::MetalSection {
                    path: ".".to_string(),
                    startup: "runtime.asm".to_string(),
                    linker: "link.ld".to_string(),
                    required_symbols: Vec::new(),
                },
                memory: None,
                features: std::collections::HashMap::new(),
                capabilities: std::collections::HashMap::new(),
                deploy: None,
                debug: None,
                test: platform::TestSection {
                    rung: platform::TestRung::Hardware,
                    target: None,
                    evidence: None,
                    debug_agent: None,
                },
                secure_boot: None,
            },
        };

        let report = hardware_report_entry(&pack);
        assert_eq!(report.verdict, Verdict::NotApplicable);
        assert!(report
            .reasons
            .contains(&"hardware-only: not emulator-tested".to_string()));
        assert!(report.label.contains("rp2350"));
        assert!(report.target.contains("armv7m-unknown-none"));
    }

    #[test]
    fn required_axes_are_core_for_qemu_targets() {
        let selection = target_selection();
        let axes = required_axes_for_selection(&selection, &HashSet::new());

        // x86_64-unknown-none has mmio_scratch → Mmio is required.
        // interrupt_source is None → Interrupt is NOT required.
        assert_eq!(
            axes,
            vec![
                CoverageAxis::Arith,
                CoverageAxis::Stack,
                CoverageAxis::Controlflow,
                CoverageAxis::CallAbi,
                CoverageAxis::MemWidth,
                CoverageAxis::Ptr,
                CoverageAxis::Locals,
                CoverageAxis::Trap,
                CoverageAxis::DeepStack,
                CoverageAxis::Mmio,
            ]
        );
    }

    #[test]
    fn required_axes_include_interrupt_only_when_qemu_has_it() {
        // ARM lm3s6965evb has interrupt_source => Interrupt is required.
        let selection = TestSelection::Target(Target::ArmV7MUnknownNone);
        let axes = required_axes_for_selection(&selection, &HashSet::new());

        assert!(axes.contains(&CoverageAxis::Interrupt));
        assert!(axes.contains(&CoverageAxis::Mmio));
    }

    #[test]
    fn required_axes_add_runtime_service_axes_from_capabilities() {
        let selection = target_selection();
        let capabilities = HashSet::from(["Channels".to_string()]);
        let axes = required_axes_for_selection(&selection, &capabilities);

        assert!(axes.contains(&CoverageAxis::Concurrency));
    }

    #[test]
    fn accumulator_verdict_fails_when_required_axis_uncovered() {
        let selection = target_selection();
        let acc = SelectionAccumulator::new(&selection, &HashSet::new());
        let report = acc.finish();

        assert_eq!(report.verdict, Verdict::Fail);
        assert_eq!(report.uncovered_axes, report.required_axes);
        assert!(report_has_failure(&QualificationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            selections: vec![report],
        }));
    }

    #[test]
    fn accumulator_verdict_passes_when_required_axes_are_covered() {
        let selection = target_selection();
        let mut acc = SelectionAccumulator::new(&selection, &HashSet::new());
        acc.covered.extend(acc.required_axes.iter().copied());
        let report = acc.finish();

        assert_eq!(report.verdict, Verdict::Pass);
        assert!(report.uncovered_axes.is_empty());
        assert!(!report_has_failure(&QualificationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            selections: vec![report],
        }));
    }

    #[test]
    fn child_report_path_is_unique_and_temp_scoped() {
        let first = child_report_path("qemu-x86_64", 0);
        let second = child_report_path("qemu-x86_64", 1);
        assert_ne!(first, second);
        assert!(first.starts_with(std::env::temp_dir().join("tyu_report")));
        let file_name = first.file_name().unwrap().to_string_lossy();
        assert!(file_name.contains(&std::process::id().to_string()));
        assert!(file_name.contains("qemu_x86_64"));
    }

    #[test]
    fn write_and_read_child_report_roundtrips() {
        let path = report_path("roundtrip");
        let _ = fs::remove_file(&path);
        let original = QualificationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            selections: vec![selection_report(Verdict::Pass)],
        };

        write_report(&path, &original).unwrap();
        let decoded = read_child_report(&target_selection(), &path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(decoded, original);
    }

    #[test]
    fn missing_child_report_is_fail_closed_error() {
        let path = report_path("missing");
        let _ = fs::remove_file(&path);

        let err = read_child_report(&target_selection(), &path).unwrap_err();

        assert!(err.contains("missing child report"));
        assert!(err.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn malformed_child_report_is_fail_closed_error() {
        let path = report_path("malformed");
        let _ = fs::remove_file(&path);
        fs::write(&path, "{not-json").unwrap();

        let err = read_child_report(&target_selection(), &path).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(err.contains("malformed child report"));
        assert!(err.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn schema_mismatched_child_report_is_fail_closed_error() {
        let path = report_path("schema");
        let _ = fs::remove_file(&path);
        let report = QualificationReport {
            schema_version: REPORT_SCHEMA_VERSION + 1,
            selections: vec![selection_report(Verdict::Pass)],
        };
        fs::write(&path, serde_json::to_string(&report).unwrap()).unwrap();

        let err = read_child_report(&target_selection(), &path).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(err.contains("schema-version-mismatch"));
        assert!(err.contains(&(REPORT_SCHEMA_VERSION + 1).to_string()));
    }

    #[test]
    fn empty_child_report_is_fail_closed_error() {
        let path = report_path("empty");
        let _ = fs::remove_file(&path);
        let report = QualificationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            selections: Vec::new(),
        };
        fs::write(&path, serde_json::to_string(&report).unwrap()).unwrap();

        let err = read_child_report(&target_selection(), &path).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(err.contains("empty child report"));
        assert!(err.contains("x86_64-unknown-none"));
    }

    #[test]
    fn synthetic_fail_selection_records_report_error() {
        let selection = target_selection();
        let report = synthetic_fail_selection(&selection, "missing child report".to_string());

        assert_eq!(report.verdict, Verdict::Fail);
        assert_eq!(report.label, "x86_64-unknown-none");
        assert_eq!(report.reasons, vec!["missing child report".to_string()]);
    }

    // -----------------------------------------------------------------------
    // Target bring-up matrix (Slice 8)
    // -----------------------------------------------------------------------

    /// Assert that every QEMU-capable target has a coherent `TargetSpec` and
    /// that the manifest declares at least one fixture per core axis.
    ///
    /// Adding a new `Target` variant MUST satisfy this matrix before it can
    /// declare `rung="qemu"` (FR-10).  The matrix is the compile-time forcing
    /// function: it proves emulator qualification, not just parsing.
    #[test]
    fn target_bringup_matrix() {
        let root = workspace_root();
        let manifest_path = root
            .join("crates")
            .join("execution-tests")
            .join("fixtures")
            .join("manifest.toml");
        let manifest = parse_manifest(&manifest_path).expect("bringup matrix: parse manifest");

        // Core axes every QEMU-capable target must cover.
        let core_axes: Vec<CoverageAxis> = CoverageAxis::ALL
            .iter()
            .copied()
            .filter(|a| a.is_core())
            .collect();

        for target in Target::ALL {
            let spec = target.spec();
            let triple = std::str::from_utf8(target.triple()).unwrap();

            // -----------------------------------------------------------------
            // TargetSpec coherence (FR-10a)
            // -----------------------------------------------------------------

            // output_format / slot_bytes / word_bits consistency.
            match spec.output_format {
                OutputFormat::Elf64 => {
                    assert_eq!(
                        spec.slot_bytes, 8,
                        "{triple}: Elf64 requires 8-byte slots, got {}",
                        spec.slot_bytes,
                    );
                    assert_eq!(spec.word_bits, 64, "{triple}: Elf64 requires word_bits=64");
                }
                OutputFormat::Elf32 => {
                    assert_eq!(
                        spec.slot_bytes, 4,
                        "{triple}: Elf32 requires 4-byte slots, got {}",
                        spec.slot_bytes,
                    );
                    assert_eq!(spec.word_bits, 32, "{triple}: Elf32 requires word_bits=32");
                }
                _ => {}
            }

            // Flat address space: pointer_bits == word_bits.
            assert_eq!(
                spec.pointer_bits, spec.word_bits,
                "{triple}: pointer_bits != word_bits",
            );

            // native_int_ty matches word_bits.
            if spec.word_bits == 64 {
                assert_eq!(
                    spec.native_int_ty, b"i64",
                    "{triple}: 64-bit target must have native_int_ty == i64",
                );
            }

            // Assembler / linker coherence.
            match spec.assembler {
                AssemblerKind::GasArm => assert!(
                    std::str::from_utf8(spec.linker)
                        .unwrap_or("")
                        .contains("arm"),
                    "{triple}: GasArm linker must contain 'arm'",
                ),
                AssemblerKind::GasRiscV => assert!(
                    std::str::from_utf8(spec.linker)
                        .unwrap_or("")
                        .contains("riscv"),
                    "{triple}: GasRiscV linker must contain 'riscv'",
                ),
                AssemblerKind::Fasm => {
                    let linker_str = std::str::from_utf8(spec.linker).unwrap_or("");
                    assert_eq!(
                        linker_str, "ld",
                        "{triple}: Fasm linker must be 'ld', got '{}'",
                        linker_str,
                    );
                }
            }

            // expected_abi_hash is deterministic (call twice, same result).
            let hash1 = spec.expected_abi_hash();
            let hash2 = spec.expected_abi_hash();
            assert_eq!(
                hash1, hash2,
                "{triple}: expected_abi_hash must be deterministic",
            );

            // -----------------------------------------------------------------
            // Core-axis fixture coverage (FR-10b)
            // -----------------------------------------------------------------

            if spec.qemu.is_none() {
                // Non-QEMU targets (e.g. hosted Linux) are not required to
                // pass the qualification suite — this gate applies only to
                // QEMU-capable bare-metal targets.
                continue;
            }

            for axis in &core_axes {
                let has_fixture = manifest.fixtures.iter().any(|f| {
                    f.axes.contains(axis)
                        && (f.targets.is_empty() || f.targets.iter().any(|t| t == triple))
                });
                assert!(
                    has_fixture,
                    "{triple}: missing core-axis fixture for '{}'. \
                     Every QEMU-capable target needs ≥1 fixture covering \
                     every core axis before it may declare rung=\"qemu\".",
                    axis.as_str(),
                );
            }
        }
    }

    /// Verify that `rung="qemu"` packs must have their target pass the
    /// core-axis qualification (FR-11).  No pack currently declares
    /// `rung="qemu"` — this test validates the gate logic and ensures
    /// it will fire if a future pack is prematurely marked.
    #[test]
    fn rung_gate_validation() {
        let root = workspace_root();
        let packs =
            platform::discover_platforms_in(&root).expect("rung gate: discover platform packs");

        for pack in &packs {
            if pack.manifest.test.rung != platform::TestRung::Qemu {
                continue;
            }
            // A pack claiming QEMU-proven status must have its resolved
            // target pass the core-axis matrix.  If we ever add such a
            // pack, this test will force the author to prove core axes.
            let selection = platform::resolve_platform_selection(&root, pack.name(), None)
                .expect("rung gate: resolve selection");
            let triple_str = std::str::from_utf8(selection.target.triple()).unwrap_or("<invalid>");
            assert!(
                selection.target.spec().qemu.is_some(),
                "pack '{}' declares rung=\"qemu\" but target {} has no QEMU spec",
                pack.name(),
                triple_str,
            );
            // The full matrix above validates all core axes — if no pack
            // has rung="qemu" this test passes vacuously, which is correct
            // (no one has claimed QEMU-proven status yet).
        }
    }
}
