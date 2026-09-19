//! Tests for build-time platform interface enforcement.

use std::fs;
use std::path::Path;
use std::process::Command;

use codegen_core::Target;
use harness_core::{parse_output, parse_records, Record};
use tyu::platform::{ensure_build_platform_interface, resolve_platform_selection};
use tyu::test_helpers::{tyu_exe, workspace_root};

fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn manifest(compiler_interface: u16) -> String {
    format!(
        r#"
[platform]
name = "demo"
compiler-interface = {compiler_interface}

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#
    )
}

fn rp2350_manifest() -> String {
    r#"
[platform]
name = "rp2350"
compiler-interface = 1

[[platform.isa]]
triple = "armv7m-unknown-none"
arch = "arm"
default = true

[[platform.isa]]
triple = "riscv32-unknown-none"
arch = "riscv"

[metal]
path = "metal/armv7m"
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "hardware"
"#
    .to_string()
}

#[test]
fn build_rejects_mismatched_platform_interface() {
    let root = std::env::temp_dir().join("tyu_platform_build_iface_mismatch");
    let _ = fs::remove_dir_all(&root);
    write_file(&root.join("runtime/demo/platform.toml"), &manifest(999));

    let err = ensure_build_platform_interface(&root, Target::X86_64UnknownNone)
        .expect_err("mismatched pack must fail");
    assert!(err.to_string().contains("E5401"), "unexpected error: {err}");
}

#[test]
fn build_accepts_matching_platform_interface() {
    let root = std::env::temp_dir().join("tyu_platform_build_iface_match");
    let _ = fs::remove_dir_all(&root);
    write_file(&root.join("runtime/demo/platform.toml"), &manifest(1));

    ensure_build_platform_interface(&root, Target::X86_64UnknownNone)
        .expect("matching pack must pass");
}

#[test]
fn resolve_platform_pack_selects_custom_platform_isa() {
    let root = std::env::temp_dir().join("tyu_platform_build_rp2350");
    let _ = fs::remove_dir_all(&root);
    write_file(
        &root.join("platforms/rp2350/platform.toml"),
        &rp2350_manifest(),
    );

    let selection = resolve_platform_selection(&root, "rp2350", Some("arm"))
        .expect("rp2350 arm selection must resolve");
    assert_eq!(selection.pack.name(), "rp2350");
    assert_eq!(selection.target, Target::ArmV7MUnknownNone);
    assert_eq!(selection.metal().path, "metal/armv7m");

    let riscv = resolve_platform_selection(&root, "rp2350", Some("riscv"))
        .expect("rp2350 riscv selection must resolve");
    assert_eq!(riscv.target, Target::RiscV32UnknownNone);
}

#[test]
fn build_rp2350_emits_image_def_and_lmod() {
    let root = std::env::temp_dir().join("tyu_platform_build_rp2350_image_def");
    let _ = fs::remove_dir_all(&root);
    let sysroot = workspace_root().join("sysroot");
    let out_dir = root.join("out");
    let main_mod = root.join("main.mod");
    write_file(
        &main_mod,
        "module Main; : main ( -- i64 ) 0 ; export { main }; end;",
    );

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let output = Command::new(tyu_exe())
        .args([
            "build",
            "--mode=static",
            "--platform=rp2350",
            "--isa=arm",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu build");
    assert!(
        output.status.success(),
        "tyu build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lmod = out_dir.join("Main.lmod");
    assert!(lmod.exists(), "final lmod was not produced");

    let exec_image = out_dir.join("image.elf");
    assert!(exec_image.exists(), "execution ELF was not produced");

    let readelf = Command::new("readelf")
        .args(["-S", &exec_image.to_string_lossy()])
        .output()
        .expect("readelf");
    assert!(readelf.status.success());
    let sections = String::from_utf8_lossy(&readelf.stdout);
    assert!(
        sections.contains(".image_def"),
        "missing .image_def section:\n{sections}"
    );
    assert!(
        sections.contains(".vectors"),
        "missing .vectors section:\n{sections}"
    );

    // Layout is owned by the pack's metal link.ld (which places .image_def
    // inside the bootrom's first-4 kB scan window); the build no longer
    // renders a parallel script for boot=image_def packs.
    let generated = out_dir.join("rp2350.link.ld");
    assert!(
        !generated.exists(),
        "build must not generate a parallel linker script for boot=image_def packs"
    );
}

#[test]
fn rp2350_uart_framed_stream_decodes_with_harness_core() {
    let mut uart = Vec::new();
    uart.extend_from_slice(&[b'V', 0x01, 0x00, 0x01]);
    uart.extend_from_slice(&[
        b'D', 0x23, 0x00, // len = 35
        0x01, // version
        0x01, // origin = IN_GUEST
        0x00, // valid
        0x00, 0x00, // trap_code
        0x00, 0x00, 0x00, 0x00, // source_line
        0x00, 0x00, 0x00, 0x00, // word_hash low
        0x00, 0x00, 0x00, 0x00, // word_hash high
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // trap_pc
        0x00, 0x00, 0x00, 0x00, // ds_depth
        0xff, 0xff, 0xff, 0xff, // ds_declared
        0x00, 0x00, // slot_count
    ]);
    uart.extend_from_slice(&[b'S', 0x01, 0x00, 0x0a]);

    let records: Vec<_> = parse_records(&uart).collect();
    assert!(matches!(records[0], Record::Version(1)));
    assert!(matches!(records[1], Record::Diag(_)));
    assert!(matches!(records[2], Record::Complete));

    let summary = parse_output(&uart);
    assert!(summary.completed);
    assert_eq!(summary.protocol_version, Some(1));
    assert_eq!(summary.diagnostics, 1);
}
