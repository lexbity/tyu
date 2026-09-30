//! `tyu doctor` — tier-scoped toolchain health checks (PLAN-RELEASE-1 S7/S8).
//!
//! Every check produces `pass | warn | fail` with an optional fix
//! suggestion. ALL tool resolution flows through
//! `toolchain::resolve_tools` / `resolve_tool_candidates` / the §6.2 probe
//! subsystem (FR-12) — doctor never re-implements resolution.
//!
//! JSON output is fixed-key `tyu.doctor/1`, hand-rolled (no serde), and
//! golden-locked byte-wise (FR-14). Exit codes (FR-15): `0` all pass, `1`
//! warnings only, `2` any fail, `3` internal error.
//!
//! S8 additions: D09 (platform-pack lint, in-process via `lint_pack`), D10
//! (the Lean/elan proof tier), and `--fix` — which auto-executes exactly one
//! action class (the user-local elan bootstrap, FR-16) and prints everything
//! else as the platform-appropriate command. `TYU_DOCTOR_FIX_DRY_RUN=1`
//! renders the fix report without executing anything (test seam).

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
    /// Platform-pack lint (§7.1 metal tier) — wraps `lint_pack` in-process.
    D09,
    /// Proof tier: the `verification/ports/lean/lean-toolchain` pin + elan +
    /// a resolvable `lean` at the pin (S8).
    D10,
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
            CheckId::D09 => "D09",
            CheckId::D10 => "D10",
            CheckId::D11 => "D11",
            CheckId::D12 => "D12",
        }
    }

    fn tier(self) -> Tier {
        match self {
            CheckId::D01 | CheckId::D02 | CheckId::D03 | CheckId::D11 | CheckId::D12 => {
                Tier::Hosted
            }
            CheckId::D04
            | CheckId::D05
            | CheckId::D06
            | CheckId::D07
            | CheckId::D08
            | CheckId::D09 => Tier::Metal,
            CheckId::D10 => Tier::Proof,
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
    if pkgs.is_empty() && !matches!(check, CheckId::D02) {
        // D09/D10 fixes are check-specific (they name the pack / use elan) and
        // are constructed inside their checks; D02 is "build the workspace".
        return FixSuggestion {
            auto: false,
            command: String::new(),
        };
    }
    FixSuggestion {
        auto: false,
        command: pkg_install(pkgs),
    }
}

/// The official elan bootstrap — the ONE `--fix` action class that is
/// auto-executed (FR-16 / Q5): user-local, no root, official installer. The
/// non-interactive form mirrors SETUP.md's `elan-init.sh -y
/// --default-toolchain none` (no default Lean toolchain is installed; the pin
/// is elan's job to install on request). `--fix` is a non-interactive CLI, so
/// a prompting installer would hang — the `-y` is mandatory, not cosmetic.
fn elan_bootstrap_command() -> String {
    "curl --proto '=https' --tlsv1.2 -sSf \
     https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh \
     | sh -s -- -y --default-toolchain none"
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The relative lean-toolchain pin file (relative to the checkout CWD).
const LEAN_PIN_REL: &str = "verification/ports/lean/lean-toolchain";

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
            // Printed, never executed (FR-16): the honest remedy is building
            // the workspace so the binaries exist in target/{debug,release}.
            r.fix = Some(FixSuggestion {
                auto: false,
                command: "cargo build --release -p langc -p tyu".to_string(),
            });
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

/// The platform-pack root D09 lints. `TYU_PLATFORM_ROOT` overrides the
/// compile-time workspace root (a documented test/override seam — identical
/// in spirit to `TYU_SYSROOT`): production resolves the checkout's own
/// `platforms/` + `runtime/`; tests point at a fabricated tree.
fn platform_root() -> PathBuf {
    if let Some(v) = std::env::var_os("TYU_PLATFORM_ROOT") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    crate::platform::workspace_root()
}

/// The metal triples D09 scopes by default: the bare-metal (QEMU-backed)
/// targets — the same set `ci.yml`'s platform-lint gate and the Q7 release
/// matrix lint. The hosted bundle (`x86_64-unknown-linux-gnu`) is deliberately
/// OUT of the metal tier's default scope: its full-surface lint carries the
/// documented pre-existing gaps (the DS-geometry exports withheld for the §7.5
/// fail-closed guard-elision design, and the legacy glue path — both pinned
/// out-of-scope by `crates/tyu/tests/platform_model_lint.rs`). Pass
/// `--platform=x86_64-unknown-linux-gnu` to lint it explicitly.
const METAL_TRIPLES: &[&str] = &[
    "x86_64-unknown-none",
    "armv7m-unknown-none",
    "riscv32-unknown-none",
];

/// D09 (§7.1 metal tier): `tyu platform lint` for every pack in scope,
/// in-process through `lint_pack` (the library entry the `platform lint`
/// command uses — no subprocess, no duplicate resolution). Scope is the
/// packs declaring `--platform`'s triple when given, else the metal-tier
/// packs (those declaring a bare-metal triple — see [`METAL_TRIPLES`]).
/// lint errors ⇒ fail (fix: re-run the lint for the pack); warnings are
/// advisory and do not fail (§7.1 pass = "lint clean").
fn check_d09(root: &Path, platform: Option<&codegen_core::Target>) -> CheckResult {
    let packs = match crate::platform::discover_platforms_in(root) {
        Ok(packs) => packs,
        Err(e) => {
            let mut r = fail_with_fix(
                CheckId::D09,
                "platform lint",
                FixSuggestion {
                    auto: false,
                    command: "review the platform pack layout (platforms/, runtime/)".to_string(),
                },
            );
            r.detail = Some(format!(
                "platform discovery failed under {}: {}",
                root.display(),
                e
            ));
            return r;
        }
    };
    let triple = platform.map(|t| std::str::from_utf8(t.triple()).unwrap_or("").to_string());
    let mut selected: Vec<String> = Vec::new();
    for pack in &packs {
        let declares = |t: &str| pack.manifest.platform.isa.iter().any(|isa| isa.triple == t);
        match triple {
            Some(ref t) if !declares(t) => continue,
            // Default (no --platform): the metal-tier scope only.
            None if !pack
                .manifest
                .platform
                .isa
                .iter()
                .any(|isa| METAL_TRIPLES.contains(&isa.triple.as_str())) =>
            {
                continue
            }
            _ => {}
        }
        selected.push(pack.name().to_string());
    }
    if selected.is_empty() {
        let mut r = pass(CheckId::D09);
        r.detail = Some(match triple {
            Some(t) => format!("no platform pack declares '{t}'"),
            None => "no platform packs in scope".to_string(),
        });
        return r;
    }

    let mut failures: Vec<String> = Vec::new();
    for name in &selected {
        match crate::platform::lint_pack(root, name, false) {
            Ok(o) if o.errors.is_empty() => {}
            Ok(o) => {
                for err in &o.errors {
                    failures.push(format!("pack={name} E{} {}", err.code, err.detail));
                }
            }
            Err(e) => failures.push(format!("pack={name}: {e}")),
        }
    }

    if failures.is_empty() {
        let mut r = pass(CheckId::D09);
        r.detail = Some(format!("{} platform pack(s) lint clean", selected.len()));
        r
    } else {
        let first = selected.first().cloned().unwrap_or_default();
        let mut r = fail_with_fix(
            CheckId::D09,
            "platform lint",
            FixSuggestion {
                auto: false,
                command: format!("tyu platform lint {first}"),
            },
        );
        r.detail = Some(failures.join("; "));
        r
    }
}

/// D10 (S8, proof tier): the `verification/ports/lean/lean-toolchain` pin
/// must parse, `elan` must be present, and `lean` must resolve. Lean's
/// version is compared against the pin — a mismatch WARNS (never fails; the
/// pin is elan's job to honor, doctor's job to report). The elan-absent fix
/// is the auto-executed `--fix` action class (FR-16).
fn check_d10(cwd: &Path) -> CheckResult {
    let Some((pin_line, pin_ver)) = parse_lean_pin(cwd) else {
        let mut r = fail_with_fix(
            CheckId::D10,
            "lean",
            FixSuggestion {
                auto: false,
                command: format!(
                    "add {LEAN_PIN_REL} to this checkout (format: leanprover/lean4:vX.Y.Z; see verification/ports/lean/README.md)"
                ),
            },
        );
        r.detail = Some(format!(
            "lean toolchain pin '{LEAN_PIN_REL}' missing or unparseable"
        ));
        return r;
    };

    if resolve_tool_candidates(&["elan"]).is_err() {
        let mut r = fail_with_fix(
            CheckId::D10,
            "lean",
            FixSuggestion {
                auto: true,
                command: elan_bootstrap_command(),
            },
        );
        r.detail = Some(format!(
            "elan not found on PATH (the proof tier needs the Lean toolchain manager; pin: {pin_line})"
        ));
        return r;
    }

    let Ok(lean_path) = resolve_tool_candidates(&["lean"]) else {
        let mut r = pass(CheckId::D10);
        r.status = Status::Warn;
        r.tool = Some("lean".to_string());
        r.detail = Some(format!(
            "elan present but lean not resolvable through the elan shims (pin: {pin_line})"
        ));
        r.fix = Some(FixSuggestion {
            auto: false,
            command: format!("elan install {pin_line}"),
        });
        return r;
    };
    let version = toolchain::probe_version(&lean_path);
    match version.as_deref() {
        Some(v) if v == pin_ver => {
            let mut r = found(pass(CheckId::D10), &lean_path, version.clone(), "path");
            r.tool = Some("lean".to_string());
            r.detail = Some(format!("matches {LEAN_PIN_REL} pin ({pin_line})"));
            r
        }
        Some(v) => {
            let mut r = found(pass(CheckId::D10), &lean_path, version.clone(), "path");
            r.status = Status::Warn;
            r.tool = Some("lean".to_string());
            r.detail = Some(format!("lean {v} != pinned {pin_ver} ({pin_line})"));
            r.fix = Some(FixSuggestion {
                auto: false,
                command: format!("elan default {pin_line}"),
            });
            r
        }
        None => {
            let mut r = found(pass(CheckId::D10), &lean_path, None, "path");
            r.status = Status::Warn;
            r.tool = Some("lean".to_string());
            r.detail = Some(format!(
                "lean resolves but its version is unparseable (pin: {pin_line})"
            ));
            r.fix = Some(FixSuggestion {
                auto: false,
                command: format!("elan default {pin_line}"),
            });
            r
        }
    }
}

/// Parse `verification/ports/lean/lean-toolchain` (`leanprover/lean4:vX.Y.Z`);
/// returns the raw line and the version (leading `v` stripped).
fn parse_lean_pin(cwd: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(cwd.join(LEAN_PIN_REL)).ok()?;
    let line = text.trim().to_string();
    let version = line.rsplit(':').next()?.trim_start_matches('v').to_string();
    if line.is_empty() || version.is_empty() {
        return None;
    }
    Some((line, version))
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
/// (D04–D09) to one target's roles (FR-11).
///
/// Returns the report plus, when `--fix` ran, the fix report (EXECUTED /
/// SUGGESTED / STILL-FAILING sections, FR-16).
pub fn run(args: &DoctorArgs, manifest: &ProjectManifest) -> (DoctorReport, Option<FixReport>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = platform_root();
    run_at(&cwd, &root, args, manifest)
}

/// Testable core of [`run`]: carries the injected CWD and platform root so
/// unit tests can fabricate a checkout and a platform-pack tree.
pub(crate) fn run_at(
    cwd: &Path,
    root: &Path,
    args: &DoctorArgs,
    manifest: &ProjectManifest,
) -> (DoctorReport, Option<FixReport>) {
    let _ = manifest;
    let mut checks = collect_checks(cwd, root, args);
    let fix = if args.fix {
        Some(apply_fixes(cwd, root, args, &mut checks))
    } else {
        None
    };
    let sysroot = crate::sysroot::resolve(None, cwd);
    let report = DoctorReport {
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
    };
    (report, fix)
}

/// The tier-scoped check list (sorted by id, ready for rendering).
fn collect_checks(cwd: &Path, root: &Path, args: &DoctorArgs) -> Vec<CheckResult> {
    let platform = args.platform;
    let mut checks = vec![
        check_d01(cwd),
        check_d02(),
        check_d03(cwd),
        check_d11(),
        check_d12(),
    ];
    if args.tier.includes_metal() {
        for id in [
            CheckId::D04,
            CheckId::D05,
            CheckId::D06,
            CheckId::D07,
            CheckId::D08,
            CheckId::D09,
        ] {
            if applies(id, platform.as_ref()) {
                checks.push(run_check(cwd, root, platform.as_ref(), id));
            }
        }
    }
    if args.tier.includes_proof() {
        checks.push(run_check(cwd, root, platform.as_ref(), CheckId::D10));
    }
    checks.sort_by_key(|c| c.id);
    checks
}

/// Run ONE check by id — the single dispatch both the initial collection and
/// the post-`--fix` re-probe use, so a fixed check is re-detected exactly as
/// it was first detected (FR-16: "re-run detection afterward").
fn run_check(
    cwd: &Path,
    root: &Path,
    platform: Option<&codegen_core::Target>,
    id: CheckId,
) -> CheckResult {
    match id {
        CheckId::D01 => check_d01(cwd),
        CheckId::D02 => check_d02(),
        CheckId::D03 => check_d03(cwd),
        CheckId::D04 => check_d04(),
        CheckId::D05 => check_tool_pair(CheckId::D05, &["ld", "nm"], CheckId::D05),
        CheckId::D06 => check_d06(),
        CheckId::D07 => check_d07(),
        CheckId::D08 => check_d08(platform),
        CheckId::D09 => check_d09(root, platform),
        CheckId::D10 => check_d10(cwd),
        CheckId::D11 => check_d11(),
        CheckId::D12 => check_d12(),
    }
}

/// The `--fix` outcome (FR-16): what was EXECUTED, what is printed as a
/// SUGGESTED command, and what is STILL-FAILING after the re-probe.
#[derive(Debug, Default)]
pub struct FixReport {
    /// Auto-executed fixes (today: the elan bootstrap), `"<id>: <command>"`.
    pub executed: Vec<String>,
    /// Printed-only suggestions, `(check id, platform command)` pairs.
    pub suggested: Vec<(String, String)>,
    /// Post-fix non-passing checks, `(check id, status)` pairs.
    pub still_failing: Vec<(String, String)>,
}

/// Apply the `--fix` policy (§Q5 / FR-16): auto-execute ONLY the elan
/// bootstrap (user-local, official installer); every other fix is collected
/// into SUGGESTED and never executed. Runs under `TYU_DOCTOR_FIX_DRY_RUN=1`
/// by printing instead of executing (a documented, tested seam — the real
/// elan download is exercised manually, not by the test suite).
fn apply_fixes(
    cwd: &Path,
    root: &Path,
    args: &DoctorArgs,
    checks: &mut [CheckResult],
) -> FixReport {
    let dry_run = std::env::var_os("TYU_DOCTOR_FIX_DRY_RUN")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let mut executed: Vec<String> = Vec::new();
    let mut suggested: Vec<(String, String)> = Vec::new();
    let mut recheck: Vec<CheckId> = Vec::new();

    for c in checks.iter() {
        if c.status == Status::Pass {
            continue;
        }
        let Some(fix) = &c.fix else { continue };
        if fix.auto {
            let id = c.id;
            if dry_run {
                executed.push(format!("[dry-run] {id}: {}", fix.command));
            } else {
                match run_shell(&fix.command) {
                    Ok(()) => executed.push(format!("{id}: {}", fix.command)),
                    Err(e) => executed.push(format!("{id}: FAILED ({e})")),
                }
            }
            recheck.push(parse_check_id(id).expect("auto-fix checks must have a CheckId factory"));
        } else if !fix.command.is_empty() {
            suggested.push((c.id.to_string(), fix.command.clone()));
        }
    }

    // FR-16 "re-run detection afterward": re-probe any check whose auto fix
    // was attempted, so the final status reflects post-fix reality.
    for id in recheck {
        let fresh = run_check(cwd, root, args.platform.as_ref(), id);
        if let Some(slot) = checks.iter_mut().find(|c| c.id == id.id()) {
            *slot = fresh;
        }
    }
    checks.sort_by_key(|c| c.id);

    let still_failing = checks
        .iter()
        .filter(|c| c.status != Status::Pass)
        .map(|c| (c.id.to_string(), c.status.name().to_string()))
        .collect();
    FixReport {
        executed,
        suggested,
        still_failing,
    }
}

fn parse_check_id(s: &str) -> Option<CheckId> {
    match s {
        "D01" => Some(CheckId::D01),
        "D02" => Some(CheckId::D02),
        "D03" => Some(CheckId::D03),
        "D04" => Some(CheckId::D04),
        "D05" => Some(CheckId::D05),
        "D06" => Some(CheckId::D06),
        "D07" => Some(CheckId::D07),
        "D08" => Some(CheckId::D08),
        "D09" => Some(CheckId::D09),
        "D10" => Some(CheckId::D10),
        "D11" => Some(CheckId::D11),
        "D12" => Some(CheckId::D12),
        _ => None,
    }
}

/// Run a fix command through the user's shell (the elan bootstrap pipeline).
/// Fixes are user-local by policy — never `sudo`, never root (NFR-6).
/// Spawn failures surface as `TyuError::Io`; a non-zero exit is a
/// `TyuError::Toolchain` carrying the exit code (the caller renders it into
/// the EXECUTED section verbatim).
fn run_shell(cmd: &str) -> Result<(), crate::error::TyuError> {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .map_err(crate::error::TyuError::Io)?;
    if status.success() {
        Ok(())
    } else {
        Err(crate::error::TyuError::Toolchain(format!(
            "fix command exited with {:?}",
            status.code()
        )))
    }
}

/// The `--fix` three-section report (FR-16), rendered to stderr by main so
/// the JSON stdout stream stays machine-clean.
pub fn render_fix(fix: &FixReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "tyu doctor --fix");
    let _ = writeln!(out, "  EXECUTED:");
    if fix.executed.is_empty() {
        let _ = writeln!(out, "    (none)");
    } else {
        for e in &fix.executed {
            let _ = writeln!(out, "    {e}");
        }
    }
    let _ = writeln!(out, "  SUGGESTED (run manually):");
    if fix.suggested.is_empty() {
        let _ = writeln!(out, "    (none)");
    } else {
        for (id, cmd) in &fix.suggested {
            let _ = writeln!(out, "    {id}: {cmd}");
        }
    }
    let _ = writeln!(out, "  STILL-FAILING:");
    if fix.still_failing.is_empty() {
        let _ = writeln!(out, "    (none)");
    } else {
        for (id, status) in &fix.still_failing {
            let _ = writeln!(out, "    {id}: {status}");
        }
    }
    out
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

    // --- D09 platform-pack lint (S8) ---------------------------------------

    /// A clean fake pack (compiler-interface matches; required symbols
    /// declared; deploy + untested rung) — the lint must pass it.
    fn write_pack(root: &Path, name: &str, compiler_interface: u16) {
        let dir = root.join("platforms").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("platform.toml"),
            format!(
                r#"[platform]
name = "{name}"
compiler-interface = {compiler_interface}

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true
expected_abi_hash = "0x50fbac4f4e87016c"

[metal]
path = "."
startup = "runtime.asm"
linker = ""

[deploy]
method = "qemu"

[test]
rung = "untested"
"#,
            ),
        )
        .unwrap();
        fs::write(
            dir.join("runtime.asm"),
            "public __lang_start\npublic __lang_trap\npublic __lang_ds_base\n\
             public __lang_ds_limit\npublic __lang_ds_high\n\
             public __lang_expected_abi_hash\n",
        )
        .unwrap();
    }

    #[test]
    fn d09_platform_lint_pass_fail_and_scope() {
        let base = temp_dir("d09");
        let root = base.join("root");
        write_pack(&root, "broken", 999); // E5401 — interface mismatch
        write_pack(&root, "clean", 1);

        // broken pack in scope ⇒ fail naming the pack, with a lint fix.
        let r = check_d09(&root, None);
        assert_eq!(r.status, Status::Fail, "a lint error must fail D09");
        let detail = r.detail.as_deref().unwrap_or("");
        assert!(
            detail.contains("broken"),
            "detail must name the failing pack: {detail}"
        );
        assert!(
            detail.contains("E5401"),
            "detail must carry the lint E-code: {detail}"
        );
        let fix = r.fix.expect("D09 fail must suggest a fix");
        assert!(!fix.auto, "D09 fix is printed, never auto-executed");
        assert_eq!(fix.command, "tyu platform lint broken");

        // only the clean pack ⇒ pass.
        fs::remove_dir_all(root.join("platforms").join("broken")).unwrap();
        let r = check_d09(&root, None);
        assert_eq!(r.status, Status::Pass, "lint-clean packs must pass");
        assert!(r.detail.as_deref().unwrap_or("").contains("lint clean"));

        // an empty root ⇒ vacuous pass (nothing in scope to lint).
        let empty = base.join("empty-root");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(check_d09(&empty, None).status, Status::Pass);

        // --platform scoping: a triple no pack declares ⇒ pass with a note.
        let r = check_d09(&root, Some(&codegen_core::Target::ArmV7MUnknownNone));
        assert_eq!(r.status, Status::Pass);
        assert!(
            r.detail
                .as_deref()
                .unwrap_or("")
                .contains("no platform pack declares"),
            "unmatched --platform must be reported, not crash"
        );
        fs::remove_dir_all(&base).unwrap();
    }

    // --- D10 proof tier (S8) -----------------------------------------------

    fn lean_checkout(base: &Path) -> PathBuf {
        let cwd = base.join("checkout");
        fs::create_dir_all(cwd.join("verification").join("ports").join("lean")).unwrap();
        fs::write(
            cwd.join("verification")
                .join("ports")
                .join("lean")
                .join("lean-toolchain"),
            "leanprover/lean4:v4.27.0\n",
        )
        .unwrap();
        cwd
    }

    #[test]
    fn d10_proof_tier_all_states() {
        let base = temp_dir("d10");

        // pin file absent ⇒ fail, printed (non-auto) fix.
        let bare = base.join("bare");
        fs::create_dir_all(&bare).unwrap();
        let r = check_d10(&bare);
        assert_eq!(r.status, Status::Fail);
        assert!(!r.fix.as_ref().unwrap().auto);

        let cwd = lean_checkout(&base);

        // elan absent ⇒ fail with the AUTO elan bootstrap fix (the one
        // --fix action class).
        let empty = base.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let _g = with_path(&empty);
        let r = check_d10(&cwd);
        assert_eq!(r.status, Status::Fail);
        let d10_fix = r.fix.expect("elan-absent must suggest the bootstrap");
        assert!(
            d10_fix.auto,
            "elan bootstrap is the auto-executed fix class"
        );
        assert!(
            d10_fix.command.contains("elan-init.sh"),
            "must name the official installer"
        );
        drop(_g);

        // elan present, lean missing ⇒ warn (never fail).
        let b1 = base.join("bin-elan");
        write_script(&b1, "elan", "#!/bin/sh\necho 'elan 5.0.0'\n");
        let _g = with_path(&b1);
        let r = check_d10(&cwd);
        assert_eq!(
            r.status,
            Status::Warn,
            "lean unresolved with elan present ⇒ warn"
        );
        assert!(!r.fix.as_ref().unwrap().auto);
        drop(_g);

        // elan + lean at the pin ⇒ pass.
        let b2 = base.join("bin-ok");
        write_script(&b2, "elan", "#!/bin/sh\necho 'elan 5.0.0'\n");
        write_script(
            &b2,
            "lean",
            "#!/bin/sh\necho 'Lean (version 4.27.0, commit abc)'\n",
        );
        let _g = with_path(&b2);
        let r = check_d10(&cwd);
        assert_eq!(r.status, Status::Pass, "lean at the pin must pass");
        assert_eq!(r.version.as_deref(), Some("4.27.0"));
        assert!(r.detail.as_deref().unwrap_or("").contains("matches"));
        drop(_g);

        // elan + lean at a DIFFERENT version ⇒ warn, never fail (the pin is
        // elan's job to honor, doctor's job to report).
        let b3 = base.join("bin-mismatch");
        write_script(&b3, "elan", "#!/bin/sh\necho 'elan 5.0.0'\n");
        write_script(
            &b3,
            "lean",
            "#!/bin/sh\necho 'Lean (version 4.16.0, commit abc)'\n",
        );
        let _g = with_path(&b3);
        let r = check_d10(&cwd);
        assert_eq!(
            r.status,
            Status::Warn,
            "version mismatch must warn, never fail"
        );
        assert_eq!(r.version.as_deref(), Some("4.16.0"));
        assert!(r.detail.as_deref().unwrap_or("").contains("!="));
        drop(_g);
        fs::remove_dir_all(&base).unwrap();
    }

    // --- --fix (FR-16, S8) ---------------------------------------------------

    #[test]
    fn fix_dry_run_executes_only_elan_bootstrap() {
        let base = temp_dir("fix");
        let cwd = lean_checkout(&base);
        let empty = base.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let _g = with_path(&empty);

        // The dry-run knob is process-global; set+restore under the ambient
        // PATH lock so concurrent tests never observe either mutation.
        let prev_dry = std::env::var_os("TYU_DOCTOR_FIX_DRY_RUN");
        std::env::set_var("TYU_DOCTOR_FIX_DRY_RUN", "1");
        let args = crate::args::DoctorArgs {
            tier: crate::args::DoctorTier::Proof,
            format: crate::args::DoctorFormat::Human,
            platform: None,
            json_out: None,
            fix: true,
        };
        let (_report, fix) = run_at(
            &cwd,
            &base.join("noplats"),
            &args,
            &crate::project::ProjectManifest::default(),
        );
        let fix = fix.expect("--fix must return a fix report");
        // EXECUTED carries the elan bootstrap, marked dry-run.
        assert!(
            fix.executed
                .iter()
                .any(|e| e.contains("D10") && e.contains("elan-init.sh")),
            "elan bootstrap must be the auto-executed fix: {:?}",
            fix.executed
        );
        assert!(
            fix.executed.iter().any(|e| e.starts_with("[dry-run]")),
            "TYU_DOCTOR_FIX_DRY_RUN=1 must never execute: {:?}",
            fix.executed
        );
        // SUGGESTED never carries D10 (its fix is the auto class).
        assert!(!fix.suggested.iter().any(|(id, _)| id == "D10"));
        // STILL-FAILING reflects the post-fix reality (still no elan).
        assert!(fix
            .still_failing
            .iter()
            .any(|(id, st)| id == "D10" && st == "fail"));
        if let Some(p) = prev_dry {
            std::env::set_var("TYU_DOCTOR_FIX_DRY_RUN", p);
        } else {
            std::env::remove_var("TYU_DOCTOR_FIX_DRY_RUN");
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
