//! Platform runtime-kind tests: the declared `[platform] kind` (hosted |
//! metal) drives the build.
//!
//! - the hosted pack (`linux-x86_64-hosted`) assembles its OWN startup unit
//!   (`[metal] startup` is honored by the builder) and the image runs
//!   natively (the hosted dynamic-entry gap);
//! - `--mode=dynamic` is refused for hosted runtimes with the intentional
//!   gate diagnostic — before any runtime unit is assembled (no
//!   missing-`dynamic_entry.asm` confusion), keyed on the DECLARED kind;
//! - metal packs keep the full dynamic flow (firmware image + on-device
//!   loader);
//! - the kind surfaces through `tyu platform lint` / `list`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tyu_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tyu"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-platform-kind-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

const RETURN_ZERO: &str = "\
module KindProbe;
: main ( -- i64 )
  0
;
end;
";

fn write_module(dir: &Path, source: &str) -> PathBuf {
    let path = dir.join("KindProbe.mod");
    fs::write(&path, source).unwrap();
    path
}

fn tyu(tag: &str, args: &[&str]) -> (String, String, bool) {
    let dir = fresh_dir(tag);
    let out = Command::new(tyu_exe())
        .args(args)
        .current_dir(&dir)
        .output()
        .expect("tyu invocation");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// The declared kind drives the runtime-unit stem: the hosted pack's startup
/// unit (`linux-x86_64-hosted.asm`, named by `[metal] startup`) assembles and
/// the linked image runs as a native process. This is the hosted-pack build
/// that previously died on the hardcoded `runtime.asm` stem.
#[test]
fn hosted_pack_builds_and_image_runs_natively() {
    // Requires fasm (the hosted x86_64 assembler), like the metal-image tests.
    if !tyu_available() {
        eprintln!("skipping: fasm or ld missing");
        return;
    }
    let dir = fresh_dir("hosted-build");
    let module = write_module(&dir, RETURN_ZERO);
    let out_dir = dir.join("out");
    let (stdout, stderr, ok) = tyu(
        "hosted-build",
        &[
            "build",
            module.to_str().unwrap(),
            "--platform=linux-x86_64-hosted",
            format!("--out-dir={}", out_dir.display()).as_str(),
        ],
    );
    assert!(
        ok,
        "hosted pack build must succeed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let image = out_dir.join("image.elf");
    assert!(image.is_file(), "hosted pack must link image.elf");

    // The image is a host-native ELF: it executes directly and exits with
    // main's return value.
    let run = Command::new(&image).status().expect("exec image.elf");
    assert_eq!(
        run.code(),
        Some(0),
        "the hosted image must exit with main's return code"
    );
}

/// `--mode=dynamic` on a hosted pack is refused by the mode gate, naming the
/// DECLARED kind — and not by the missing dynamic entry unit (the gate runs
/// before any runtime assembly).
#[test]
fn hosted_pack_dynamic_mode_gated_on_declared_kind() {
    let dir = fresh_dir("hosted-dyn");
    let module = write_module(&dir, RETURN_ZERO);
    let (_, stderr, ok) = tyu(
        "hosted-dyn",
        &[
            "build",
            module.to_str().unwrap(),
            "--platform=linux-x86_64-hosted",
            "--mode=dynamic",
            format!("--out-dir={}", dir.join("out").display()).as_str(),
        ],
    );
    assert!(!ok, "hosted --mode=dynamic must be refused");
    assert!(
        stderr.contains("requires a metal runtime") && stderr.contains("declares kind=hosted"),
        "the gate must name the declared kind\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("dynamic runtime entry unit"),
        "the low-level missing-entry diagnostic must not surface\nstderr:\n{stderr}"
    );
}

/// A bare host-native target has no firmware flow either: the gate fires
/// before assembly, not as a missing-file error.
#[test]
fn bare_hosted_target_dynamic_mode_gated() {
    let dir = fresh_dir("bare-dyn");
    let module = write_module(&dir, RETURN_ZERO);
    let (_, stderr, ok) = tyu(
        "bare-dyn",
        &[
            "build",
            module.to_str().unwrap(),
            "--target=x86_64-unknown-linux-gnu",
            "--mode=dynamic",
            format!("--out-dir={}", dir.join("out").display()).as_str(),
        ],
    );
    assert!(!ok, "hosted-target --mode=dynamic must be refused");
    assert!(
        stderr.contains("bare-metal (QEMU-capable) target"),
        "the gate must name the firmware-flow requirement\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("dynamic runtime entry unit"),
        "the low-level missing-entry diagnostic must not surface\nstderr:\n{stderr}"
    );
}

/// Metal packs keep the full dynamic flow: the firmware image links with the
/// dynamic entry, the on-device loader, and the modpack.
#[test]
fn metal_pack_dynamic_mode_links_firmware() {
    if !metal_tools_available() {
        eprintln!("skipping: fasm/ld/qemu tooling missing");
        return;
    }
    let dir = fresh_dir("metal-dyn");
    let module = write_module(&dir, RETURN_ZERO);
    let out_dir = dir.join("out");
    let (stdout, stderr, ok) = tyu(
        "metal-dyn",
        &[
            "build",
            module.to_str().unwrap(),
            "--platform=x86_64-unknown-none",
            "--mode=dynamic",
            format!("--out-dir={}", out_dir.display()).as_str(),
        ],
    );
    assert!(
        ok,
        "metal --mode=dynamic must link the firmware image\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(out_dir.join("image.elf").is_file());
}

/// The kind-aware lint: every in-tree pack lints green under the declared
/// kind (metal packs need startup+linker; the hosted pack needs no linker
/// script; the engine-hosted wasm pack needs no native units).
#[test]
fn declared_kind_lints_all_packs() {
    for pack in [
        "x86_64-unknown-none",
        "armv7m-unknown-none",
        "riscv32-unknown-none",
        "rp2350",
        "wasm32-unknown-unknown",
        "linux-x86_64-hosted",
    ] {
        let (stdout, stderr, ok) = tyu("lint", &["platform", "lint", pack]);
        assert!(
            ok,
            "pack '{pack}' must lint green\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

/// The kind surfaces in the operator reporting (`tyu platform list`).
#[test]
fn platform_list_reports_declared_kind() {
    let (stdout, _, ok) = tyu("list", &["platform", "list"]);
    assert!(ok);
    assert!(
        stdout.contains("linux-x86_64-hosted | kind=hosted |"),
        "the hosted pack must report kind=hosted\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("x86_64-unknown-none | kind=metal |"),
        "the metal pack must report kind=metal\nstdout:\n{stdout}"
    );
}

fn tyu_available() -> bool {
    Command::new("fasm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Command::new("ld")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn metal_tools_available() -> bool {
    tyu_available()
        && Command::new("qemu-system-x86_64")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        && Command::new("cargo")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}
