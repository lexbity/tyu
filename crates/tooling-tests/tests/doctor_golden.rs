//! `tyu doctor` golden-locked JSON (PLAN-RELEASE-1 S7, FR-14).
//!
//! Runs the REAL `tyu` binary against a synthetic probe environment: a
//! fabricated `PATH` pointing at a per-fixture fakebin (generated scripts with
//! FIXED version strings), CWD = the workspace checkout, and byte-compares the
//! `--format=json` output to `test-goldens/doctor/<fixture>.json`.
//!
//! Absolute paths cannot be golden-stable, so the output is NORMALIZED before
//! the byte compare (the only deviation from a literally byte-wise compare):
//! the workspace root becomes `@ROOT@` and the per-fixture fakebin dir becomes
//! `@FAKEBIN@`. Semantics/statuses/versions/keys are compared byte-exactly.
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

/// Assemble one fixture's fakebin. `which` selects the tools that resolve.
fn fakebin(dir: &Path, tools: &[&str]) -> PathBuf {
    let channel = pinned_channel();
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
    dir.to_path_buf()
}

/// Run `tyu doctor --tier=all --format=json` against the fixture, with the
/// fakebin as the ONLY PATH (the child inherits it; the parent's PATH is
/// untouched, so concurrent suite tests never observe our PATH).
fn doctor_json(fakebin: &Path) -> Output {
    let tyu = bin::resolve("tyu");
    Command::new(tyu)
        .current_dir(workspace_root())
        .env("PATH", fakebin)
        .args(["doctor", "--tier=all", "--format=json"])
        .output()
        .expect("spawn tyu doctor")
}

/// Normalize absolute paths and platform-specific fix commands out of the
/// golden output: the checkout becomes `@ROOT@`, the fixture fakebin dir
/// `@FAKEBIN@`, and the (OS-detected) fix command `@FIX-CMD@` — the fix's
/// SHAPE (auto flag, presence) stays golden-locked; the exact command string
/// is OS data verified by doctor's fix-table unit test.
fn normalize(raw: &str, fakebin: &Path) -> String {
    let rooted = raw
        .replace(&workspace_root().display().to_string(), "@ROOT@")
        .replace(&fakebin.display().to_string(), "@FAKEBIN@");
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

fn check_fixture(name: &str, tools: &[&str], want_exit: i32) {
    ensure_fresh_tyu_bin();
    let bin_dir = temp_dir(name);
    let fakebin = fakebin(&bin_dir, tools);
    let out = doctor_json(&fakebin);
    assert_eq!(
        out.status.code(),
        Some(want_exit),
        "{name}: tyu doctor exit {} (want {}): {}",
        out.status,
        want_exit,
        String::from_utf8_lossy(&out.stderr)
    );
    let golden_abs = golden_path(name);
    let normalized = normalize(&String::from_utf8_lossy(&out.stdout), &fakebin);
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
];
const HOSTED_ONLY: &[&str] = &["rustc", "rustup", "git"];
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
];

/// Healthy hosted+metal environment: everything passes.
#[test]
fn golden_healthy() {
    check_fixture("healthy", ALL, 0);
}

/// Hosted healthy, every metal tool missing → D04–D08 all fail (exit 2).
#[test]
fn golden_missing_metal() {
    check_fixture("missing-metal", HOSTED_ONLY, 2);
}

/// Only the x86_64 QEMU installed → D08 warns (exit 1), the rest pass.
#[test]
fn golden_mixed_warn() {
    check_fixture("mixed-warn", WARN_QEMU, 1);
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
    let out = doctor_json(&fakebin);
    let normalized = normalize(&String::from_utf8_lossy(&out.stdout), &fakebin);
    let expected = fs::read_to_string(golden_path("healthy")).unwrap();
    assert_ne!(
        normalized, expected,
        "a perturbed probe environment must change the doctor JSON"
    );
    fs::remove_dir_all(&bin_dir).unwrap();
}
