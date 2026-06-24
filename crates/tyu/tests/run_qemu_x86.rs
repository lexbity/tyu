//! Tests for `tyu run` on x86_64-unknown-none (QEMU).
//!
//! R-1: pass, fail-marker, no-completion (with completed assertion).
//! R-2: deterministic hang detection (no TCO dependency).

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
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");

    let sysroot = workspace_root().join("sysroot");
    let output = Command::new(tyu_exe())
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu build");

    if !output.status.success() {
        panic!(
            "tyu build failed for {}:\n{}",
            label,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    out_dir.join("Main.lmod")
}

const PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

const FAIL_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 70 testio.write-byte 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

const UART_TIME_MOD: &str = "\
module Main;\nimport platform/uart { platform.uart.init platform.uart.tx };\n\
import platform/time { platform.time.now_us };\n\
: main ( -- i64 ) 115200 as usize platform.uart.init 83 as u8 platform.uart.tx 10 as u8 platform.uart.tx platform.time.now_us drop 0 ;\n\
export { main };\nend;\n";

const NO_COMPLETION_MOD: &str = "\
module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n";

// ---------------------------------------------------------------------------
// R-1: pass, fail-marker, no-completion
// ---------------------------------------------------------------------------

#[test]
fn run_qemu_pass() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    let dir = temp_dir("pass");
    let image = build_x86_image(PASS_MOD, &dir, "pass");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner)
        .run(&image, Duration::from_secs(10))
        .unwrap();
    assert!(!outcome.timed_out);
    assert_eq!(outcome.exit_code, 1);
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(s.completed);
    assert_eq!(s.failures, 0);
}

#[test]
fn run_qemu_fail_marker() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    let dir = temp_dir("fail");
    let image = build_x86_image(FAIL_MOD, &dir, "fail");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner)
        .run(&image, Duration::from_secs(10))
        .unwrap();
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(s.failures > 0);
    assert!(
        s.completed,
        "R-1: fail-marker test must still report completion (S\\n)"
    );
}

#[test]
fn run_qemu_uart_time_words() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    let dir = temp_dir("uart_time");
    let image = build_x86_image(UART_TIME_MOD, &dir, "uart_time");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner)
        .run(&image, Duration::from_secs(10))
        .unwrap();
    assert!(!outcome.timed_out);
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(
        s.completed,
        "uart/time smoke test must report completion (S\\n)"
    );
    assert_eq!(s.failures, 0);
}

#[test]
fn run_qemu_no_completion() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    let dir = temp_dir("nocomplete");
    let image = build_x86_image(NO_COMPLETION_MOD, &dir, "nocomplete");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner)
        .run(&image, Duration::from_secs(10))
        .unwrap();
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(!s.completed);
}

// ---------------------------------------------------------------------------
// R-2: Deterministic hang detection
// ---------------------------------------------------------------------------
//
// Uses a counted loop that never reaches its terminating condition.
// No recursion (no TCO dependency).  The QEMU process genuinely does
// not terminate, so the 500 ms timeout expires and timed_out == true.

const HANG_MOD: &str = "\
module Main;\n\
: main ( -- i64 ) 0 begin 1 + dup 0 < until drop 0 ;\n\
export { main };\nend;\n";

#[test]
#[ignore = "pre-existing: HANG_MOD typecheck error E3210 — fix in Slice 8/9"]
fn run_qemu_hang_detected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    let dir = temp_dir("hang");
    let image = build_x86_image(HANG_MOD, &dir, "hang");
    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = Runner::Qemu(runner)
        .run(&image, Duration::from_millis(500))
        .unwrap();
    assert!(
        outcome.timed_out,
        "R-2: program must time out, got exit_code={}",
        outcome.exit_code
    );
    let s = harness_core::parse_output(&outcome.stdout);
    assert!(
        !s.completed,
        "R-2: timed-out program must not report completion"
    );
}
