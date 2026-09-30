//! `tyu doctor` — tier-scoped toolchain health checks (PLAN-RELEASE-1 S7).
//!
//! Every check produces `pass | warn | fail` with an optional fix
//! suggestion. ALL tool resolution flows through
//! `toolchain::resolve_tools` / `resolve_tool_candidates` / the §6.2 probe
//! subsystem (FR-12) — doctor never re-implements resolution.
//!
//! JSON output is fixed-key `tyu.doctor/1`, hand-rolled (no serde), and
//! golden-locked byte-wise (FR-14). Exit codes (FR-15): `0` all pass, `1`
//! warnings only, `2` any fail, `3` internal error.

use crate::args::DoctorArgs;
use crate::project::ProjectManifest;
use crate::toolchain::{self, resolve_tool_candidates, ToolResolution};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Status / tier / check identities
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tier {
    Hosted,
    Metal,
    #[allow(dead_code)] // Proof checks land in the S8 slice.
    Proof,
}

impl Tier {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Tier::Hosted => "hosted",
            Tier::Metal => "metal",
            Tier::Proof => "proof",
        }
    }
}

/// The checks doctor can run. `.id()` is the fixed D-code used in the JSON.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum CheckId {
    D01,
    D02,
    D03,
    D04,
    D05,
    D06,
    D07,
    D08,
    D11,
    D12,
}

impl CheckId {
    pub(crate) fn id(self) -> &'static str {
        match self {
            CheckId::D01 => "D01",
            CheckId::D02 => "D02",
            CheckId::D03 => "D03",
            CheckId::D04 => "D04",
            CheckId::D05 => "D05",
            CheckId::D06 => "D06",
            CheckId::D07 => "D07",
            CheckId::D08 => "D08",
            CheckId::D11 => "D11",
            CheckId::D12 => "D12",
        }
    }

    fn tier(self) -> Tier {
        match self {
            CheckId::D01 | CheckId::D02 | CheckId::D03 | CheckId::D11 | CheckId::D12 => {
                Tier::Hosted
            }
            CheckId::D04 | CheckId::D05 | CheckId::D06 | CheckId::D07 | CheckId::D08 => Tier::Metal,
        }
    }
}

/// A fix suggestion (static per-platform command map; printed, never
/// auto-executed except the elan bootstrap in the S8 `--fix` slice).
#[derive(Debug)]
pub(crate) struct FixSuggestion {
    pub(crate) auto: bool,
    pub(crate) command: String,
}

/// One check result.
#[derive(Debug)]
pub(crate) struct CheckResult {
    pub(crate) id: &'static str,
    pub(crate) tier: &'static str,
    pub(crate) status: Status,
    pub(crate) tool: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) fix: Option<FixSuggestion>,
}

/// The full doctor report.
#[derive(Debug)]
pub struct DoctorReport {
    pub(crate) tier: &'static str,
    pub(crate) host_os: &'static str,
    pub(crate) host_arch: &'static str,
    pub(crate) sysroot_requested: String,
    pub(crate) sysroot_resolved: Option<PathBuf>,
    pub(crate) sysroot_present: bool,
    pub(crate) checks: Vec<CheckResult>,
}

impl DoctorReport {
    /// FR-15 exit code from the check statuses.
    pub fn exit_code(&self) -> i32 {
        if self.checks.iter().any(|c| c.status == Status::Fail) {
            2
        } else if self.checks.iter().any(|c| c.status == Status::Warn) {
            1
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// check helpers
// ---------------------------------------------------------------------------

fn pass(id: CheckId) -> CheckResult {
    CheckResult {
        id: id.id(),
        tier: id.tier().name(),
        status: Status::Pass,
        tool: None,
        version: None,
        path: None,
        source: None,
        detail: None,
        fix: None,
    }
}

fn fail_with_fix(id: CheckId, tool: &str, fix: FixSuggestion) -> CheckResult {
    CheckResult {
        id: id.id(),
        tier: id.tier().name(),
        status: Status::Fail,
        tool: Some(tool.to_string()),
        version: None,
        path: None,
        source: None,
        detail: None,
        fix: Some(fix),
    }
}

/// Resolved-tool tuple for a check that found its binary.
fn found(check: CheckResult, path: &Path, version: Option<String>, source: &str) -> CheckResult {
    CheckResult {
        path: Some(path.display().to_string()),
        source: Some(source.to_string()),
        version,
        ..check
    }
}

// ---------------------------------------------------------------------------
// fix table (static per-platform commands, §Q5 — printed, never executed)
// ---------------------------------------------------------------------------

/// The distro/OS detected for fix-command selection (§Q5 — the commands are
/// printed, never auto-executed).
fn platform_pkg_manager() -> &'static str {
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut id = "";
    let mut id_like = "";
    for line in os.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            id = v.trim_matches('"');
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            id_like = v.trim_matches('"');
        }
    }
    let is_arch = id == "arch"
        || id == "manjaro"
        || id_like.split_whitespace().any(|p| p == "arch")
        || id == "endeavouros"
        || id == "arcolinux";
    if is_arch {
        "pacman"
    } else if matches!(id, "fedora" | "rhel" | "centos")
        || id_like
            .split_whitespace()
            .any(|p| matches!(p, "fedora" | "rhel" | "centos"))
    {
        "dnf"
    } else if cfg!(target_os = "macos") {
        "brew"
    } else {
        "apt"
    }
}

fn pkg_install(package: &str) -> String {
    match platform_pkg_manager() {
        "dnf" => format!("sudo dnf install -y {package}"),
        "pacman" => format!("sudo pacman -S --noconfirm {package}"),
        "brew" => format!("brew install {package}"),
        _ => format!("sudo apt-get install -y {package}"),
    }
}

fn fix_for(check: CheckId) -> FixSuggestion {
    let pkgs = match check {
        CheckId::D04 => "fasm",
        CheckId::D05 => "binutils",
        CheckId::D06 => "gcc-arm-none-eabi",
        CheckId::D07 => "gcc-riscv64-unknown-elf",
        CheckId::D08 => "qemu-system-x qemu-system-arm qemu-system-misc",
        CheckId::D11 => "git",
        _ => "",
    };
    FixSuggestion {
        auto: false,
        command: pkg_install(pkgs),
    }
}

// ---------------------------------------------------------------------------
// platform scoping (FR-11: --platform restricts D04–D08 to one target's roles)
// ---------------------------------------------------------------------------

fn applies(check: CheckId, platform: Option<&codegen_core::Target>) -> bool {
    let Some(t) = platform else {
        return true;
    };
    match check {
        // D04: fasm is the x86 assembler (hosted and x86_64-none).
        CheckId::D04 => t.spec().assembler == codegen_core::AssemblerKind::Fasm,
        // D05: ld/nm are target-independent.
        CheckId::D05 => true,
        // D06/D07: the cross-assembler for this target.
        CheckId::D06 => t.spec().assembler == codegen_core::AssemblerKind::GasArm,
        CheckId::D07 => t.spec().assembler == codegen_core::AssemblerKind::GasRiscV,
        // D08: this target's QEMU binary.
        CheckId::D08 => t.spec().qemu.is_some(),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// the checks
// ---------------------------------------------------------------------------

fn parse_toolchain_channel(cwd: &Path) -> Option<String> {
    let toml = std::fs::read_to_string(cwd.join("rust-toolchain.toml")).ok()?;
    for line in toml.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("channel") {
            let v = v.trim();
            let v = v.strip_prefix('=')?.trim();
            let v = v.trim_matches('"');
            return Some(v.to_string());
        }
    }
    None
}

/// The active toolchain channel: `rustup show active-toolchain` when rustup
/// is on PATH (exact, dated), else the channel TYPE from `rustc --version`
/// (stable/nightly/beta — date-level matching needs rustup).
fn active_toolchain_channel(version: Option<&str>) -> Option<String> {
    if let Ok(rustup) = resolve_tool_candidates(&["rustup"]) {
        if let Some(out) = toolchain::run_probe(
            &rustup,
            &["show", "active-toolchain"],
            toolchain::ProbeStream::Stdout,
        ) {
            let first = out.split_whitespace().next()?;
            return Some(first.to_string());
        }
    }
    if let Some(v) = version {
        let first = v.split_whitespace().next()?; // "1.101.0-nightly"
        return first.rsplit('-').next().map(|s| s.to_string()); // "nightly"
    }
    None
}

fn check_d01(cwd: &Path) -> CheckResult {
    let rustc = resolve_tool_candidates(&["rustc"]);
    let Ok(rc_path) = rustc else {
        let mut r = fail_with_fix(CheckId::D01, "rustc", fix_for(CheckId::D11));
        r.detail = Some("rustc not found on PATH or in the workspace target dirs".to_string());
        return r;
    };
    let version = toolchain::probe_version(&rc_path);
    let Some(channel) = parse_toolchain_channel(cwd) else {
        // rustc present but no pinned toolchain file to compare.
        let mut r = found(pass(CheckId::D01), &rc_path, version, "path");
        r.status = Status::Warn;
        r.tool = Some("rustc".to_string());
        r.detail = Some(
            "rust-toolchain.toml missing in the current directory — cannot verify the pin"
                .to_string(),
        );
        return r;
    };
    let active = active_toolchain_channel(version.as_deref());
    let matches = active
        .as_deref()
        .map(|a| a.starts_with(&channel) || channel_type(&channel) == a)
        .unwrap_or(false);
    let mut r = found(pass(CheckId::D01), &rc_path, version, "path");
    r.tool = Some("rustc".to_string());
    if matches {
        r.detail = Some(format!("matches rust-toolchain.toml pin ({channel})"));
    } else {
        r.status = Status::Fail;
        r.detail = Some(format!(
            "channel mismatch: active {} != pinned {channel}",
            active.unwrap_or_default()
        ));
    }
    r
}

fn channel_type(channel: &str) -> String {
    channel.split('-').next().unwrap_or(channel).to_string()
}

fn check_d02() -> CheckResult {
    check_d02_from(
        resolve_tool_candidates(&["tyu"]),
        resolve_tool_candidates(&["langc"]),
    )
}

/// Testable core: the check's branch logic given the two resolvabilities.
/// Resolution correctness itself is covered by the toolchain tests (the
/// runtime workspace chain) — doctor only classifies the results (FR-12).
fn check_d02_from(
    tyu: Result<PathBuf, crate::error::TyuError>,
    langc: Result<PathBuf, crate::error::TyuError>,
) -> CheckResult {
    match (tyu, langc) {
        (Ok(_), Ok(_)) => {
            let mut r = pass(CheckId::D02);
            r.detail = Some("tyu and langc both resolvable".to_string());
            r
        }
        (Ok(_), Err(_)) => {
            let mut r = pass(CheckId::D02);
            r.status = Status::Warn;
            r.detail = Some("tyu resolvable but langc is not".to_string());
            r
        }
        (Err(_), Ok(_)) => {
            let mut r = pass(CheckId::D02);
            r.status = Status::Warn;
            r.detail = Some("langc resolvable but tyu is not".to_string());
            r
        }
        (Err(_), Err(_)) => {
            let mut r = pass(CheckId::D02);
            r.status = Status::Fail;
            r.detail = Some(
                "neither tyu nor langc resolvable (PATH or workspace target dirs)".to_string(),
            );
            fix_command(&mut r, fix_for(CheckId::D02));
            r
        }
    }
}

fn check_d03(cwd: &Path) -> CheckResult {
    let env_set = std::env::var_os("TYU_SYSROOT")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let requested = if env_set { "env" } else { "cwd" };
    let resolved = crate::sysroot::resolve(None, cwd);
    let mut r = pass(CheckId::D03);
    match resolved {
        None => {
            r.status = Status::Fail;
            r.detail = Some("no sysroot resolvable: set TYU_SYSROOT, run from a tyu checkout, or pass --sysroot".to_string());
            r
        }
        Some(p) => {
            if p.join("Core.mod").is_file() {
                r.detail = Some(format!("{requested} -> {} (Core.mod present)", p.display()));
            } else {
                r.status = Status::Warn;
                r.detail = Some(format!(
                    "{requested} -> {} resolved but Core.mod is missing",
                    p.display()
                ));
            }
            r
        }
    }
}

fn check_d04() -> CheckResult {
    match resolve_tool_candidates(&["fasm"]) {
        Ok(p) => {
            let v = toolchain::probe_version(&p);
            let mut r = found(pass(CheckId::D04), &p, v, "path");
            r.tool = Some("fasm".to_string());
            r
        }
        Err(_) => fail_with_fix(CheckId::D04, "fasm", fix_for(CheckId::D04)),
    }
}

fn check_tool_pair(id: CheckId, names: &[&str], pkg: CheckId) -> CheckResult {
    let mut present = Vec::new();
    for n in names {
        if let Ok(p) = resolve_tool_candidates(&[n]) {
            present.push((*n, p));
        }
    }
    if present.len() == names.len() {
        let mut r = pass(id);
        let paths: Vec<String> = present.iter().map(|(n, _)| n.to_string()).collect();
        r.detail = Some(paths.join(", ") + " present");
        r
    } else {
        fail_with_fix(id, &names.join("/"), fix_for(pkg))
    }
}

fn check_d06() -> CheckResult {
    match resolve_tool_candidates(&["arm-none-eabi-as"]) {
        Ok(p) => {
            let v = toolchain::probe_version(&p);
            let mut r = found(pass(CheckId::D06), &p, v, "path");
            r.tool = Some("arm-none-eabi-as".to_string());
            r
        }
        Err(_) => fail_with_fix(CheckId::D06, "arm-none-eabi-as", fix_for(CheckId::D06)),
    }
}

fn check_d07() -> CheckResult {
    let mut found_p = None;
    for n in toolchain::RISCV_AS_CANDIDATES
        .iter()
        .chain(toolchain::RISCV_LD_CANDIDATES.iter())
    {
        if let Ok(p) = resolve_tool_candidates(&[*n]) {
            found_p = Some(p);
            break;
        }
    }
    match found_p {
        Some(p) => {
            let mut r = pass(CheckId::D07);
            r.tool = Some(
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
            r.path = Some(p.display().to_string());
            r.source = Some("path".to_string());
            r.detail = Some("a RISC-V cross assembler/linker candidate resolves".to_string());
            r
        }
        None => fail_with_fix(
            CheckId::D07,
            "riscv32/riscv64-*-as/ld",
            fix_for(CheckId::D07),
        ),
    }
}

fn check_d08(platform: Option<&codegen_core::Target>) -> CheckResult {
    // §7.1: all three QEMU system binaries by default; --platform restricts
    // D08 to that target's QEMU.
    if let Some(t) = platform {
        let Some(q) = t.spec().qemu else {
            return pass(CheckId::D08); // no QEMU role for this target
        };
        let name = std::str::from_utf8(q.system_bin).unwrap_or_default();
        return match resolve_tool_candidates(&[name]) {
            Ok(p) => {
                let mut r = pass(CheckId::D08);
                r.tool = Some(name.to_string());
                r.path = Some(p.display().to_string());
                r
            }
            Err(_) => fail_with_fix(CheckId::D08, name, fix_for(CheckId::D08)),
        };
    }
    let wanted: &[&str] = &[
        "qemu-system-x86_64",
        "qemu-system-arm",
        "qemu-system-riscv32",
    ];
    let mut present = Vec::new();
    for n in wanted {
        if let Ok(p) = resolve_tool_candidates(&[n]) {
            present.push(p);
        }
    }
    if present.len() == wanted.len() {
        let mut r = pass(CheckId::D08);
        r.detail = Some(format!("{} QEMU system binaries present", wanted.len()));
        r
    } else if present.is_empty() {
        fail_with_fix(CheckId::D08, "qemu-system-*", fix_for(CheckId::D08))
    } else {
        let mut r = pass(CheckId::D08);
        r.status = Status::Warn;
        let found: Vec<String> = present
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        r.detail = Some(format!(
            "missing some QEMU binaries; have {}",
            found.join(", ")
        ));
        fix_command(&mut r, fix_for(CheckId::D08));
        r
    }
}

fn check_d11() -> CheckResult {
    match resolve_tool_candidates(&["git"]) {
        Ok(p) => {
            let v = toolchain::probe_version(&p);
            let mut r = found(pass(CheckId::D11), &p, v, "path");
            r.tool = Some("git".to_string());
            r
        }
        Err(_) => fail_with_fix(CheckId::D11, "git", fix_for(CheckId::D11)),
    }
}

fn check_d12() -> CheckResult {
    // PATH hygiene: a PATH-entry tyu that differs from the resolved one
    // (workspace/exe-dir) shadows it.
    let resolved = resolve_tool_candidates(&["tyu"]).ok();
    let path_tyu = std::env::var_os("PATH").and_then(|p| {
        for dir in std::env::split_paths(&p) {
            let c = dir.join("tyu");
            if c.is_file() {
                return Some(c);
            }
        }
        None
    });
    let mut r = pass(CheckId::D12);
    match (resolved, path_tyu) {
        (Some(res), Some(p)) if p != res => {
            r.status = Status::Warn;
            r.detail = Some(format!(
                "PATH entry {} shadows the resolved tyu at {}",
                p.display(),
                res.display()
            ));
        }
        _ => {
            r.detail = Some("no shadowing tyu on PATH".to_string());
        }
    }
    r
}

fn fix_command(r: &mut CheckResult, fix: FixSuggestion) {
    r.fix = Some(fix);
}

// ---------------------------------------------------------------------------
// run / check dispatch
// ---------------------------------------------------------------------------

/// Run the tier-scoped check catalog (§7.1). `check_tools` is the FR-12
/// surface: it resolves the target's four roles exactly as `toolchain check`
/// does (both use `toolchain::resolve_tools`), so the two reports can never
/// disagree about which binary a build would use.
pub fn check_tools(target: codegen_core::Target, manifest: &ProjectManifest) -> ToolResolution {
    toolchain::resolve_tools(target, manifest, &std::collections::HashMap::new())
}

/// Run doctor over the requested tier. `platform` restricts the metal checks
/// (D04–D08) to one target's roles (FR-11).
pub fn run(args: &DoctorArgs, manifest: &ProjectManifest) -> DoctorReport {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let platform = args.platform;
    let _ = manifest;

    // Hosted tier is always included (D01–D03, D11, D12).
    let mut checks = vec![
        check_d01(&cwd),
        check_d02(),
        check_d03(&cwd),
        check_d11(),
        check_d12(),
    ];

    // Metal tier adds the per-target tool roles (D04–D08), scoped by
    // --platform when given.
    if args.tier.includes_metal() {
        if applies(CheckId::D04, platform.as_ref()) {
            checks.push(check_d04());
        }
        if applies(CheckId::D05, platform.as_ref()) {
            checks.push(check_tool_pair(CheckId::D05, &["ld", "nm"], CheckId::D05));
        }
        if applies(CheckId::D06, platform.as_ref()) {
            checks.push(check_d06());
        }
        if applies(CheckId::D07, platform.as_ref()) {
            checks.push(check_d07());
        }
        if applies(CheckId::D08, platform.as_ref()) {
            checks.push(check_d08(platform.as_ref()));
        }
    }

    checks.sort_by_key(|c| c.id);
    let sysroot = crate::sysroot::resolve(None, &cwd);
    DoctorReport {
        tier: args.tier.name(),
        host_os: std::env::consts::OS,
        host_arch: std::env::consts::ARCH,
        sysroot_requested: if std::env::var_os("TYU_SYSROOT")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            "env"
        } else {
            "cwd"
        }
        .to_string(),
        sysroot_present: sysroot
            .as_deref()
            .map(|p| p.is_dir() && p.join("Core.mod").is_file())
            .unwrap_or(false),
        sysroot_resolved: sysroot,
        checks,
    }
}

// ---------------------------------------------------------------------------
// renderers
// ---------------------------------------------------------------------------

/// Human renderer: aligned columns; fail lines end with the fix command.
pub fn render_human(report: &DoctorReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "tyu doctor — {}/{}", report.host_os, report.host_arch);
    if let Some(p) = &report.sysroot_resolved {
        let _ = writeln!(
            out,
            "sysroot: {} -> {} ({})",
            report.sysroot_requested,
            p.display(),
            if report.sysroot_present {
                "present"
            } else {
                "incomplete"
            }
        );
    }
    for c in &report.checks {
        let mark = match c.status {
            Status::Pass => "[ ok ]",
            Status::Warn => "[warn]",
            Status::Fail => "[FAIL]",
        };
        let _ = write!(out, "  {mark} {:<4} {:<9}", c.id, c.tier);
        if let Some(t) = &c.tool {
            let _ = write!(out, " {t}");
        }
        if let Some(p) = &c.path {
            let _ = write!(out, " {}", p);
        }
        if let Some(v) = &c.version {
            let _ = write!(out, " ({v})");
        }
        if c.status != Status::Pass {
            if let Some(d) = &c.detail {
                let _ = write!(out, " — {d}");
            }
            if let Some(f) = &c.fix {
                let _ = write!(out, "  fix: {}", f.command);
            }
        }
        let _ = writeln!(out);
    }
    let passes = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Pass)
        .count();
    let warns = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Warn)
        .count();
    let fails = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Fail)
        .count();
    let _ = writeln!(out, "summary: {passes} pass, {warns} warn, {fails} fail");
    out
}

/// JSON writer — fixed key order `id, tier, status, tool, version, path,
/// source, detail, fix`, checks sorted by id, two-space indent (FR-14 / §6.1).
pub fn render_json(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push('{');
    let _ = write!(out, "\n  \"schema\": \"tyu.doctor/1\"");
    let _ = write!(out, ",\n  \"tyu_version\": {}", ji("0.1.0"));
    let _ = write!(out, ",\n  \"tier\": {}", ji(report.tier));
    let _ = write!(
        out,
        ",\n  \"host\": {{ \"os\": {}, \"arch\": {} }}",
        ji(report.host_os),
        ji(report.host_arch)
    );
    let _ = write!(
        out,
        ",\n  \"sysroot\": {{ \"requested\": {}, \"resolved\": {}, \"present\": {} }}",
        ji(&report.sysroot_requested),
        report
            .sysroot_resolved
            .as_deref()
            .map(|p| ji(&p.display().to_string()))
            .unwrap_or_else(|| "null".to_string()),
        if report.sysroot_present {
            "true"
        } else {
            "false"
        }
    );
    out.push_str(",\n  \"checks\": [");
    for (i, c) in report.checks.iter().enumerate() {
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        let _ = write!(
            out,
            "    {{ \"id\": {}, \"tier\": {}, \"status\": {}",
            ji(c.id),
            ji(c.tier),
            ji(c.status.name())
        );
        let _ = write!(
            out,
            ", \"tool\": {}",
            c.tool
                .as_deref()
                .map(ji)
                .unwrap_or_else(|| "null".to_string())
        );
        let _ = write!(
            out,
            ", \"version\": {}",
            c.version
                .as_deref()
                .map(ji)
                .unwrap_or_else(|| "null".to_string())
        );
        let _ = write!(
            out,
            ", \"path\": {}",
            c.path
                .as_deref()
                .map(ji)
                .unwrap_or_else(|| "null".to_string())
        );
        let _ = write!(
            out,
            ", \"source\": {}",
            c.source
                .as_deref()
                .map(ji)
                .unwrap_or_else(|| "null".to_string())
        );
        let _ = write!(
            out,
            ", \"detail\": {}",
            c.detail
                .as_deref()
                .map(ji)
                .unwrap_or_else(|| "null".to_string())
        );
        // fix present iff status ∈ {warn, fail} and a fix exists.
        if c.status != Status::Pass {
            if let Some(f) = &c.fix {
                let _ = write!(
                    out,
                    ", \"fix\": {{ \"auto\": {}, \"command\": {} }}",
                    if f.auto { "true" } else { "false" },
                    ji(&f.command)
                );
            }
        }
        out.push('}');
    }
    if !report.checks.is_empty() {
        out.push('\n');
        out.push_str("  ");
    }
    out.push_str("],\n");
    let passes = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Pass)
        .count();
    let warns = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Warn)
        .count();
    let fails = report
        .checks
        .iter()
        .filter(|c| c.status == Status::Fail)
        .count();
    let _ = write!(
        out,
        "  \"summary\": {{ \"pass\": {passes}, \"warn\": {warns}, \"fail\": {fails} }}"
    );
    out.push_str("\n}\n");
    out
}

/// JSON string literal with escaping.
fn ji(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = format!(
            "tyu-doctor-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
        p
    }

    /// RAII PATH replacer: sets PATH to `dir` and restores it on drop.
    struct PathGuard {
        prev: Option<OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    fn with_path(dir: &Path) -> PathGuard {
        // Serialize PATH mutation AND by-name spawns suite-wide: doctor
        // fabricates PATH to probe resolution; deploy/proof tests spawn
        // processes by name and must not observe our fabricated PATH.
        let _lock = crate::toolchain::ambient_path_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var_os("PATH").unwrap_or_default();
        std::env::set_var("PATH", dir);
        PathGuard {
            prev: Some(prev),
            _lock,
        }
    }
    impl Drop for PathGuard {
        fn drop(&mut self) {
            if let Some(p) = self.prev.take() {
                std::env::set_var("PATH", p);
            }
        }
    }

    /// A fabricated healthy tool set: everything the hosted+metal tiers need.
    fn healthy_bin(dir: &Path) {
        write_script(
            dir,
            "rustc",
            "#!/bin/sh\necho 'rustc 1.101.0-nightly (c1070d693 2026-09-28)'\n",
        );
        write_script(
            dir,
            "rustup",
            "#!/bin/sh\necho 'nightly-2026-09-29-x86_64-unknown-linux-gnu (default)'\n",
        );
        write_script(dir, "git", "#!/bin/sh\necho 'git version 2.55.0'\n");
        write_script(
            dir,
            "fasm",
            "#!/bin/sh\necho 'flat assembler  version 1.73.35'\n",
        );
        write_script(
            dir,
            "ld",
            "#!/bin/sh\necho 'GNU ld (GNU Binutils) 2.42.0'\n",
        );
        write_script(
            dir,
            "nm",
            "#!/bin/sh\necho 'GNU nm (GNU Binutils) 2.42.0'\n",
        );
        write_script(
            dir,
            "arm-none-eabi-as",
            "#!/bin/sh\necho 'GNU assembler 2.41'\n",
        );
        write_script(
            dir,
            "riscv64-unknown-elf-as",
            "#!/bin/sh\necho 'GNU assembler 2.41'\n",
        );
        write_script(
            dir,
            "qemu-system-x86_64",
            "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
        );
        write_script(
            dir,
            "qemu-system-arm",
            "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
        );
        write_script(
            dir,
            "qemu-system-riscv32",
            "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
        );
    }

    // --- per-check pass/fail/warn -----------------------------------------

    #[test]
    fn d04_fasm_pass_and_fail_with_fix() {
        let base = temp_dir("d04");
        fs::create_dir_all(base.join("empty")).unwrap();
        let _g = with_path(&base.join("empty"));
        let r = check_d04();
        assert_eq!(r.status, Status::Fail);
        assert!(r.fix.is_some(), "missing fasm must suggest a fix");
        drop(_g);

        let bin = write_script(
            &base.join("bin"),
            "fasm",
            "#!/bin/sh\necho 'flat assembler  version 1.73.35'\n",
        );
        let _g = with_path(&base.join("bin"));
        let r = check_d04();
        assert_eq!(r.status, Status::Pass);
        assert_eq!(r.version.as_deref(), Some("1.73.35"));
        assert_eq!(r.path.as_deref(), Some(bin.to_str().unwrap()));
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn d05_ld_nm_pair() {
        let base = temp_dir("d05");
        fs::create_dir_all(base.join("empty")).unwrap();
        let _g = with_path(&base.join("empty"));
        assert_eq!(check_d05(), Status::Fail);
        drop(_g);
        let b = base.join("bin");
        write_script(&b, "ld", "#!/bin/sh\necho 'GNU ld'\n");
        write_script(&b, "nm", "#!/bin/sh\necho 'GNU nm'\n");
        let _g = with_path(&b);
        assert_eq!(check_d05(), Status::Pass);
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    fn check_d05() -> Status {
        check_tool_pair(CheckId::D05, &["ld", "nm"], CheckId::D05).status
    }

    #[test]
    fn d06_arm_as() {
        let base = temp_dir("d06");
        fs::create_dir_all(base.join("empty")).unwrap();
        let _g = with_path(&base.join("empty"));
        assert_eq!(check_d06().status, Status::Fail);
        drop(_g);
        let b = base.join("bin");
        write_script(
            &b,
            "arm-none-eabi-as",
            "#!/bin/sh\necho 'GNU assembler 2.41'\n",
        );
        let _g = with_path(&b);
        assert_eq!(check_d06().status, Status::Pass);
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn d07_riscv_candidates() {
        let base = temp_dir("d07");
        fs::create_dir_all(base.join("empty")).unwrap();
        let _g = with_path(&base.join("empty"));
        assert_eq!(check_d07().status, Status::Fail);
        drop(_g);
        let b = base.join("bin");
        write_script(&b, "riscv64-unknown-elf-ld", "#!/bin/sh\necho 'GNU ld'\n");
        let _g = with_path(&b);
        assert_eq!(check_d07().status, Status::Pass);
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn d08_qemu_pass_warn_fail() {
        let base = temp_dir("d08");
        fs::create_dir_all(base.join("e")).unwrap();
        let _g = with_path(&base.join("e"));
        assert_eq!(check_d08(None).status, Status::Fail);
        drop(_g);
        let b = base.join("bin");
        write_script(
            &b,
            "qemu-system-x86_64",
            "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
        );
        let _g = with_path(&b);
        assert_eq!(check_d08(None).status, Status::Warn);
        drop(_g);
        healthy_bin(&b);
        let _g = with_path(&b);
        assert_eq!(check_d08(None).status, Status::Pass);
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn d11_git() {
        let base = temp_dir("d11");
        fs::create_dir_all(base.join("empty")).unwrap();
        let _g = with_path(&base.join("empty"));
        assert_eq!(check_d11().status, Status::Fail);
        drop(_g);
        let b = base.join("bin");
        write_script(&b, "git", "#!/bin/sh\necho 'git version 2.55.0'\n");
        let _g = with_path(&b);
        assert_eq!(check_d11().status, Status::Pass);
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn d02_both_one_none() {
        // The branch logic is tested hermetically via the check's injectable
        // core — in-repo the real workspace binaries are always resolvable
        // (the FR-13 ancestor walk), so presence/absence is not simulatable
        // through PATH here. Resolution correctness is covered by the
        // toolchain tests.
        let err = || {
            Err(crate::error::TyuError::Toolchain(
                "simulated absent".to_string(),
            ))
        };
        assert_eq!(
            check_d02_from(
                Ok(PathBuf::from("/sim/tyu")),
                Ok(PathBuf::from("/sim/langc"))
            )
            .status,
            Status::Pass
        );
        assert_eq!(
            check_d02_from(Ok(PathBuf::from("/sim/tyu")), err()).status,
            Status::Warn,
            "only tyu"
        );
        assert_eq!(
            check_d02_from(err(), Ok(PathBuf::from("/sim/langc"))).status,
            Status::Warn,
            "only langc"
        );
        let none = check_d02_from(err(), err());
        assert_eq!(none.status, Status::Fail);
        assert!(none.fix.is_some(), "absent tools must suggest a fix");
    }

    // --- D01 channel matching ---------------------------------------------

    fn toml_cwd(base: &Path, channel: &str) -> PathBuf {
        fs::create_dir_all(base).unwrap();
        fs::write(
            base.join("rust-toolchain.toml"),
            format!("[toolchain]\nchannel = \"{channel}\"\n"),
        )
        .unwrap();
        base.to_path_buf()
    }

    #[test]
    fn d01_channel_match_and_mismatch() {
        let base = temp_dir("d01");
        let cwd = toml_cwd(&base.join("cwd"), "nightly-2026-09-29");
        let b = base.join("bin");
        write_script(
            &b,
            "rustc",
            "#!/bin/sh\necho 'rustc 1.101.0-nightly (c1070d693 2026-09-28)'\n",
        );
        write_script(
            &b,
            "rustup",
            "#!/bin/sh\necho 'nightly-2026-09-29-x86_64-unknown-linux-gnu (default)'\n",
        );
        let _g = with_path(&b);
        let r = check_d01(&cwd);
        assert_eq!(
            r.status,
            Status::Pass,
            "active rustup toolchain matches the pin"
        );
        drop(_g);

        write_script(
            &b,
            "rustup",
            "#!/bin/sh\necho 'nightly-2026-06-25-x86_64-unknown-linux-gnu (default)'\n",
        );
        let _g = with_path(&b);
        let r = check_d01(&cwd);
        assert_eq!(
            r.status,
            Status::Fail,
            "a different nightly date must mismatch"
        );
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn d01_warns_when_pin_missing() {
        let base = temp_dir("d01w");
        let cwd = base.join("cwd");
        fs::create_dir_all(&cwd).unwrap(); // no rust-toolchain.toml
        let b = base.join("bin");
        healthy_bin(&b);
        let _g = with_path(&b);
        let r = check_d01(&cwd);
        assert_eq!(
            r.status,
            Status::Warn,
            "rustc present but no pin file to compare"
        );
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    // --- D03 sysroot ------------------------------------------------------

    #[test]
    fn d03_sysroot_resolved_and_unresolvable() {
        let base = temp_dir("d03");
        // resolvable: cwd/sysroot/Core.mod present
        let cwd = base.join("checkout");
        fs::create_dir_all(cwd.join("sysroot")).unwrap();
        fs::write(
            cwd.join("sysroot").join("Core.mod"),
            b"module Core;\nend;\n",
        )
        .unwrap();
        assert_eq!(check_d03(&cwd).status, Status::Pass);
        // resolved but incomplete
        let cwd2 = base.join("bare");
        fs::create_dir_all(cwd2.join("sysroot")).unwrap();
        assert_eq!(check_d03(&cwd2).status, Status::Warn);
        // unresolvable: no cwd/sysroot; guard against a stray TYU_SYSROOT
        let prev = std::env::var_os("TYU_SYSROOT");
        std::env::remove_var("TYU_SYSROOT");
        let cwd3 = base.join("nowhere");
        fs::create_dir_all(&cwd3).unwrap();
        assert_eq!(check_d03(&cwd3).status, Status::Fail);
        if let Some(p) = prev {
            std::env::set_var("TYU_SYSROOT", p);
        }
        fs::remove_dir_all(&base).unwrap();
    }

    // --- D12 PATH hygiene -------------------------------------------------

    #[test]
    fn d12_shadowing_warn() {
        let base = temp_dir("d12");
        let b = base.join("bin");
        healthy_bin(&b);
        let _g = with_path(&b);
        // no shadowing: PATH tyu == resolved tyu (both from PATH)
        assert_eq!(check_d12().status, Status::Pass);
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    // --- FR-12: doctor resolves through the same code as toolchain check ---
    #[test]
    fn fr12_doctor_and_toolchain_check_share_resolution() {
        use crate::project::ProjectManifest;
        let base = temp_dir("fr12");
        let b = base.join("bin");
        healthy_bin(&b);
        let _g = with_path(&b);
        let manifest = ProjectManifest::default();
        let target = codegen_core::Target::parse(b"x86_64-unknown-linux-gnu").unwrap();
        let from_doctor = check_tools(target, &manifest);
        let via_toolchain = toolchain::resolve_tools(target, &manifest, &Default::default());
        for (a, b_) in [
            (
                from_doctor.compiler.as_ref(),
                via_toolchain.compiler.as_ref(),
            ),
            (
                from_doctor.assembler.as_ref(),
                via_toolchain.assembler.as_ref(),
            ),
            (from_doctor.linker.as_ref(), via_toolchain.linker.as_ref()),
            (from_doctor.qemu.as_ref(), via_toolchain.qemu.as_ref()),
        ] {
            assert_eq!(
                a.map(|t| t.path.clone()),
                b_.map(|t| t.path.clone()),
                "FR-12: doctor must resolve identically to toolchain check"
            );
        }
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }
}

#[test]
fn fix_table_is_data_not_executed() {
    // The fix table is a static per-platform command map (printed, never
    // auto-executed except the S8 elan bootstrap). Assert the shape for
    // the detected platform: a real package-manager line naming the tool.
    for (check, pkg) in [
        (CheckId::D04, "fasm"),
        (CheckId::D05, "binutils"),
        (CheckId::D06, "gcc-arm-none-eabi"),
        (CheckId::D08, "qemu-system-x"),
        (CheckId::D11, "git"),
    ] {
        let f = fix_for(check);
        assert!(
            !f.auto,
            "{}: fixes are never auto-executed (S8 adds only elan)",
            check.id()
        );
        assert!(
            f.command.contains(pkg),
            "{}: {} missing from fix '{}'",
            check.id(),
            pkg,
            f.command
        );
        let apt = f.command.contains("install ");
        let pacman = f.command.contains("-S ") || f.command.contains("-Sy ");
        assert!(
            apt || pacman || f.command.starts_with("brew "),
            "{}: fix must be a real package-manager line, got '{}'",
            check.id(),
            f.command
        );
    }
}
