//! Tests for `tyu run` on x86_64-unknown-none (QEMU).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use codegen_core::Target;
use tyu::runner::Runner;
use tyu::test_helpers::*;

fn build_x86_image(src: &str, dir: &PathBuf, label: &str) -> PathBuf {
    let main_mod = dir.join(format!("{}.mod", label));
    std::fs::write(&main_mod, src).unwrap();
    let out_dir = dir.join(label);

    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root()).args(["build", "-q", "-p", "langc"]).status().expect("cargo build");
    assert!(status.success(), "cargo build failed");

    let sysroot = workspace_root().join("sysroot");
    let output = Command::new(tyu_exe())
        .args(["build", "--target=x86_64-unknown-none",
               &format!("--sysroot={}", sysroot.display()),
               &format!("--out-dir={}", out_dir.display()),
               &main_mod.to_string_lossy()])
        .output().expect("tyu build");

    if !output.status.success() {
        panic!("tyu build failed for {}:\n{}", label, String::from_utf8_lossy(&output.stderr));
    }
    out_dir.join("image.elf")
}

const PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

const FAIL_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 70 testio.write-byte 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

const NO_COMPLETION_MOD: &str = "\
module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";

const HANG_MOD: &str = "\
module Main;\n: main ( -- i64 ) main ;\nexport { main };\nend;\n";

#[test]
fn run_qemu_pass() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) { return; }
    let dir = temp_dir("pass");
    let image = build_x86_image(PASS_MOD, &dir, "pass");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner).run(&image, Duration::from_secs(10)).unwrap();
    assert!(!outcome.timed_out);
    assert_eq!(outcome.exit_code, 1);
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(s.completed);
    assert_eq!(s.failures, 0);
}

#[test]
fn run_qemu_fail_marker() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) { return; }
    let dir = temp_dir("fail");
    let image = build_x86_image(FAIL_MOD, &dir, "fail");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner).run(&image, Duration::from_secs(10)).unwrap();
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(s.failures > 0);
}

#[test]
fn run_qemu_no_completion() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) { return; }
    let dir = temp_dir("nocomplete");
    let image = build_x86_image(NO_COMPLETION_MOD, &dir, "nocomplete");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner).run(&image, Duration::from_secs(10)).unwrap();
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(!s.completed);
}

#[test]
fn run_qemu_hang_detected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) { return; }
    let dir = temp_dir("hang");
    let image = build_x86_image(HANG_MOD, &dir, "hang");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner).run(&image, Duration::from_millis(500)).unwrap();
    assert!(outcome.timed_out);
}
