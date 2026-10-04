//! `tyu doctor` golden-locked JSON (PLAN-RELEASE-1 S7/S8, FR-14).
//!
//! Runs the REAL `tyu` binary against a synthetic probe environment: a
//! fabricated `PATH` pointing at a per-fixture fakebin (generated scripts with
//! FIXED version strings), CWD = the workspace checkout, and byte-compares the
//! `--format=json` output to `test-goldens/doctor/<fixture>.json`.
//!
//! Absolute paths cannot be golden-stable, so the output is NORMALIZED before
//! the byte compare (the only deviation from a literally byte-wise compare):
//! the workspace root becomes `@ROOT@`, the per-fixture fakebin dir becomes
//! `@FAKEBIN@`, and the (S8) fake platform root `@PLATROOT@`; per-environment
//! fix commands become `@FIX-CMD@`. Semantics/statuses/versions/keys are
//! compared byte-exactly.
//!
//! S8 fixtures add the full catalog: D09 (platform lint — the real checkout's
//! five packs must lint clean, so a broken pack anywhere in the tree turns the
//! goldens red — the honesty loop) and D10 (the Lean pin + elan/lean).
//!
//! Regenerate the goldens deliberately with:
//!   TYU_REGEN_DOCTOR_GOLDENS=1 cargo test --release -p tooling-tests \
//!       --test doctor_golden
//! (the mutation proof below asserts the writer must change the goldens too).

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use common::bin;

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

/// Rebuild `tyu` at THIS test run's profile before any fixture runs.
///
/// The golden test executes the prebuilt binary (common::bin::resolve). A
/// stale binary silently voids the mutation-proof guarantee: the suite would
/// "prove" the writer against whatever binary last occupied target/<profile>.
/// CI ordering (build before goldens) masks this; a local
/// `cargo test -p tooling-tests` does not. Spawning cargo here mirrors
/// `common::ensure_bins()` (same precedent, same lock-contention tradeoff) —
/// one quick build, once per test-binary run, before any fixture spawns.
static TYU_BIN_ONCE: Once = Once::new();
fn ensure_fresh_tyu_bin() {
    TYU_BIN_ONCE.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(workspace_root())
            .args(["build", "-q", "-p", "tyu"]);
        if !cfg!(debug_assertions) {
            // Release-profile test run: the resolver (common::bin) prefers
            // target/release, so rebuild the release binary, not debug.
            cmd.arg("--release");
        }
        let status = cmd.status().expect("cargo build -p tyu for doctor goldens");
        assert!(status.success(), "building tyu for doctor goldens failed");
    });
}

// ---------------------------------------------------------------------------
// fake probe environment
// ---------------------------------------------------------------------------

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    fs::write(&p, body).unwrap();
    let mut perms = fs::metadata(&p).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&p, perms).unwrap();
    p
}

fn temp_dir(tag: &str) -> PathBuf {
    let unique = format!(
        "tyu-doctor-golden-{}-{}-{:?}",
        tag,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::env::temp_dir().join(unique)
}

/// Read the committed pinned toolchain channel so the fixture's fake `rustup`
/// prints exactly it (deterministic match regardless of the pin date).
fn pinned_channel() -> String {
    let toml = fs::read_to_string(workspace_root().join("rust-toolchain.toml")).unwrap();
    toml.lines()
        .find(|l| l.trim_start().starts_with("channel"))
        .and_then(|l| l.split('"').nth(1))
        .expect("rust-toolchain.toml channel line")
        .to_string()
}

/// The Lean version the port's `lean-toolchain` pin demands — the fake `lean`
/// prints exactly it for the pass fixtures, a different one for the
/// mismatch fixtures (D10 warn, never fail).
fn lean_pin_version() -> String {
    let text = fs::read_to_string(workspace_root().join("verification/ports/lean/lean-toolchain"))
        .unwrap();
    text.trim()
        .rsplit(':')
        .next()
        .expect("lean-toolchain pin is owner:repo:vX.Y.Z")
        .trim_start_matches('v')
        .to_string()
}

/// Assemble one fixture's fakebin. `which` selects the tools that resolve.
fn fakebin(dir: &Path, tools: &[&str]) -> PathBuf {
    let channel = pinned_channel();
    let lean_ver = lean_pin_version();
    let mk = |name: &str, body: &str| {
        if tools.contains(&name) {
            write_script(dir, name, body);
        }
    };
    mk(
        "rustc",
        "#!/bin/sh\necho 'rustc 1.101.0-nightly (c1070d693 2026-09-28)'\n",
    );
    mk(
        "rustup",
        &format!("#!/bin/sh\necho '{channel}-x86_64-unknown-linux-gnu (default)'\n"),
    );
    mk("git", "#!/bin/sh\necho 'git version 2.55.0'\n");
    mk(
        "fasm",
        "#!/bin/sh\necho 'flat assembler  version 1.73.35'\n",
    );
    mk("ld", "#!/bin/sh\necho 'GNU ld (GNU Binutils) 2.42.0'\n");
    mk("nm", "#!/bin/sh\necho 'GNU nm (GNU Binutils) 2.42.0'\n");
    mk("arm-none-eabi-as", "#!/bin/sh\necho 'GNU assembler 2.41'\n");
    mk(
        "riscv64-unknown-elf-as",
        "#!/bin/sh\necho 'GNU assembler 2.41'\n",
    );
    mk(
        "qemu-system-x86_64",
        "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
    );
    mk(
        "qemu-system-arm",
        "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
    );
    mk(
        "qemu-system-riscv32",
        "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
    );
    mk("elan", "#!/bin/sh\necho 'elan 5.0.0'\n");
    mk(
        "lean",
        &format!("#!/bin/sh\necho 'Lean (version {lean_ver}, commit abc)'\n"),
    );
    dir.to_path_buf()
}

/// Run `tyu doctor --tier=all --format=json` against the fixture, with the
/// fakebin as the ONLY PATH (the child inherits it; the parent's PATH is
/// untouched, so concurrent suite tests never observe our PATH).
fn doctor_json(fakebin: &Path, extra_env: &[(&str, &str)]) -> Output {
    let tyu = bin::resolve("tyu");
    let mut cmd = Command::new(tyu);
    cmd.current_dir(workspace_root())
        .env("PATH", fakebin)
        // Hermetic by default: an ambient TYU_PLATFORM_ROOT must not leak
        // into fixtures that exercise the real checkout's D09.
        .env_remove("TYU_PLATFORM_ROOT");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.args(["doctor", "--tier=all", "--format=json"])
        .output()
        .expect("spawn tyu doctor")
}

/// Normalize absolute paths and platform-specific fix commands out of the
/// golden output: the checkout becomes `@ROOT@`, the fixture fakebin dir
/// `@FAKEBIN@`, the fake platform root `@PLATROOT@`, and the (OS-detected)
/// fix command `@FIX-CMD@` — the fix's SHAPE (auto flag, presence) stays
/// golden-locked; the exact command string is OS data verified by doctor's
/// fix-table unit test.
fn normalize(raw: &str, fakebin: &Path, platroot: Option<&Path>) -> String {
    let mut rooted = raw
        .replace(&workspace_root().display().to_string(), "@ROOT@")
        .replace(&fakebin.display().to_string(), "@FAKEBIN@");
    if let Some(p) = platroot {
        rooted = rooted.replace(&p.display().to_string(), "@PLATROOT@");
    }
    normalize_fix_commands(&rooted)
}

/// Replace every `"command": "<...>"` with `"command": "@FIX-CMD@"`.
fn normalize_fix_commands(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let needle = "\"command\": \"";
    while let Some(start) = rest.find(needle) {
        out.push_str(&rest[..start + needle.len()]);
        rest = &rest[start + needle.len()..];
        match rest.find('"') {
            Some(end) => {
                out.push_str("@FIX-CMD@");
                rest = &rest[end..];
            }
            None => {
                out.push_str(rest);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn golden_path(fixture: &str) -> PathBuf {
    workspace_root()
        .join("test-goldens")
        .join("doctor")
        .join(format!("{fixture}.json"))
}

/// A deliberate-lint-error platform pack tree for the `metal-lint-fail`
/// fixture: `compiler-interface = 999` trips E5401 on load.
fn broken_platform_tree(dir: &Path) {
    let pack = dir.join("platforms").join("broken");
    fs::create_dir_all(&pack).unwrap();
    fs::write(
        pack.join("platform.toml"),
        r#"[platform]
name = "broken"
compiler-interface = 999

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
    )
    .unwrap();
    fs::write(
        pack.join("runtime.asm"),
        "public __lang_start\npublic __lang_trap\npublic __lang_ds_base\n\
         public __lang_ds_limit\npublic __lang_ds_high\n\
         public __lang_expected_abi_hash\n",
    )
    .unwrap();
}

fn check_fixture(
    name: &str,
    tools: &[&str],
    lean_override: Option<&str>,
    platroot: Option<&Path>,
    want_exit: i32,
) {
    ensure_fresh_tyu_bin();
    let bin_dir = temp_dir(name);
    let fakebin = fakebin(&bin_dir, tools);
    if let Some(v) = lean_override {
        write_script(
            &fakebin,
            "lean",
            &format!("#!/bin/sh\necho 'Lean (version {v}, commit abc)'\n"),
        );
    }
    let mut envs: Vec<(&str, &str)> = Vec::new();
    if let Some(p) = platroot {
        envs.push(("TYU_PLATFORM_ROOT", p.to_str().unwrap()));
    }
    let out = doctor_json(&fakebin, &envs);
    assert_eq!(
        out.status.code(),
        Some(want_exit),
        "{name}: tyu doctor exit {} (want {}): {}",
        out.status,
        want_exit,
        String::from_utf8_lossy(&out.stderr)
    );
    let golden_abs = golden_path(name);
    let normalized = normalize(&String::from_utf8_lossy(&out.stdout), &fakebin, platroot);
    if std::env::var_os("TYU_REGEN_DOCTOR_GOLDENS").is_some() {
        fs::create_dir_all(golden_abs.parent().unwrap()).unwrap();
        fs::write(&golden_abs, normalized.as_bytes()).unwrap();
        eprintln!("regen: wrote {name} golden to {}", golden_abs.display());
    } else {
        let expected = fs::read_to_string(&golden_abs).unwrap_or_else(|e| {
            panic!(
                "missing golden {} ({e}) — reject a writer change without a golden update; \
                     regen with TYU_REGEN_DOCTOR_GOLDENS=1",
                golden_abs.display()
            )
        });
        assert_eq!(
            normalized, expected,
            "{name}: doctor JSON drifted from the committed golden (regen deliberately with \
             TYU_REGEN_DOCTOR_GOLDENS=1)"
        );
    }
    fs::remove_dir_all(&bin_dir).unwrap();
}

// ---------------------------------------------------------------------------
// fixtures
// ---------------------------------------------------------------------------

const ALL: &[&str] = &[
    "rustc",
    "rustup",
    "git",
    "fasm",
    "ld",
    "nm",
    "arm-none-eabi-as",
    "riscv64-unknown-elf-as",
    "qemu-system-x86_64",
    "qemu-system-arm",
    "qemu-system-riscv32",
    "elan",
    "lean",
];
const HOSTED_ONLY: &[&str] = &["rustc", "rustup", "git"];
/// Hoted + complete metal + elan (no lean) — isolates D10's warn/fail.
const PROOF_TOOLS: &[&str] = &[
    "rustc",
    "rustup",
    "git",
    "fasm",
    "ld",
    "nm",
    "arm-none-eabi-as",
    "riscv64-unknown-elf-as",
    "qemu-system-x86_64",
    "qemu-system-arm",
    "qemu-system-riscv32",
    "elan",
];
const WARN_QEMU: &[&str] = &[
    "rustc",
    "rustup",
    "git",
    "fasm",
    "ld",
    "nm",
    "arm-none-eabi-as",
    "riscv64-unknown-elf-as",
    "qemu-system-x86_64",
    "elan",
];

/// Healthy hosted+metal+proof environment: everything passes (D09 lints the
/// real checkout's five packs clean; D10's lean is at the committed pin).
#[test]
fn golden_healthy() {
    check_fixture("healthy", ALL, None, None, 0);
}

/// Hosted healthy, every metal tool missing → D04–D08 all fail; elan absent
/// → D10 fails with the auto bootstrap fix (exit 2).
#[test]
fn golden_missing_metal() {
    check_fixture("missing-metal", HOSTED_ONLY, None, None, 2);
}

/// Only the x86_64 QEMU installed → D08 warns; lean at a DIFFERENT version
/// → D10 warns (exit 1).
#[test]
fn golden_mixed_warn() {
    check_fixture("mixed-warn", WARN_QEMU, Some("4.16.0"), None, 1);
}

/// Proof tier specifically: hosted healthy, elan present, lean at a DIFFERENT
/// version → D10 warns, nothing fails (exit 1).
#[test]
fn golden_proof_tier() {
    check_fixture("proof-tier", PROOF_TOOLS, Some("4.16.0"), None, 1);
}

/// All tools healthy except a deliberately broken platform pack under
/// `TYU_PLATFORM_ROOT` → D09 fails with the lint fix (exit 2).
#[test]
fn golden_metal_lint_fail() {
    ensure_fresh_tyu_bin();
    let root = temp_dir("metal-lint");
    broken_platform_tree(&root);
    check_fixture("metal-lint-fail", ALL, None, Some(&root), 2);
    fs::remove_dir_all(&root).unwrap();
}

/// A deliberately altered writer must be caught: mutate a fake tool's version
/// and assert the output no longer matches the committed golden.
#[test]
fn golden_mutation_is_detected() {
    let bin_dir = temp_dir("mutation");
    let fakebin = fakebin(&bin_dir, ALL);
    // poké the committed fake's fixed version by overriding with a differer.t
    fs::write(
        fakebin.join("fasm"),
        "#!/bin/sh\necho 'flat assembler  version 9.99.99'\n",
    )
    .unwrap();
    let out = doctor_json(&fakebin, &[]);
    let normalized = normalize(&String::from_utf8_lossy(&out.stdout), &fakebin, None);
    let expected = fs::read_to_string(golden_path("healthy")).unwrap();
    assert_ne!(
        normalized, expected,
        "a perturbed probe environment must change the doctor JSON"
    );
    fs::remove_dir_all(&bin_dir).unwrap();
}
