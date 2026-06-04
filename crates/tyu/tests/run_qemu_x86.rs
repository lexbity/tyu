//! Tests for `tyu run` on x86_64-unknown-none (QEMU).
//!
//! Requires `langc`, `fasm`, `ld`, and `qemu-system-x86_64` in PATH.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use codegen_core::Target;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn tyu_exe() -> PathBuf {
    workspace_root().join("target").join("debug").join("tyu")
}

fn tool_available(name: &str) -> bool {
    Command::new("which").arg(name).output()
        .map(|o| o.status.success()).unwrap_or(false)
}

fn require_tools(tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools.iter().filter(|t| !tool_available(t)).copied().collect();
    if missing.is_empty() {
        return true;
    }
    if std::env::var("CI").is_ok() {
        panic!("Required tools not available under CI: {}", missing.join(", "));
    }
    eprintln!("SKIP: required tools not available ({})", missing.join(", "));
    false
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_run_qemu_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A pass test: returns 0 and prints S\n.
const PASS_MOD: &str = "\
module Main;
import platform/testio { testio.write-byte };
: main ( -- i64 )
  83 testio.write-byte 10 testio.write-byte 0 ;
export { main };
end;
";

/// A FAIL test: emits F byte.
const FAIL_MOD: &str = "\
module Main;
import platform/testio { testio.write-byte };
: main ( -- i64 )
  70 testio.write-byte 83 testio.write-byte 10 testio.write-byte 0 ;
export { main };
end;
";

/// A NO_COMPLETION test: exits without S\n.
const NO_COMPLETION_MOD: &str = "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
";

/// A hanging test: infinite loop.
const HANG_MOD: &str = "\
module Main;
: main ( -- i64 ) main ;
export { main };
end;
";

fn build_x86_image(src: &str, dir: &PathBuf, label: &str) -> PathBuf {
    let main_mod = dir.join(format!("{}.mod", label));
    std::fs::write(&main_mod, src).unwrap();
    let out_dir = dir.join(label);

    // Build langc first.
    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    let sysroot = workspace_root().join("sysroot");
    let tyu = tyu_exe();
    let output = Command::new(&tyu)
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("tyu build failed for {}:\n{}", label, stderr);
    }

    out_dir.join("image.elf")
}

#[test]
fn run_qemu_pass() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }

    let dir = temp_dir("pass");
    let image = build_x86_image(PASS_MOD, &dir, "pass");

    let runner = codegen_core::Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = tyu::runner::Runner::Qemu(runner)
        .run(&image, Duration::from_secs(10))
        .expect("QEMU run should succeed");

    assert!(!outcome.timed_out, "should not time out");
    assert_eq!(outcome.exit_code, 1, "QEMU IsaDebugExit pass code = 1");

    let summary = harness_core::parse_output(&outcome.stdout);
    assert!(summary.completed, "S\\n marker should be present");
    assert_eq!(summary.failures, 0, "no failures");
}

#[test]
fn run_qemu_fail_marker() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }

    let dir = temp_dir("fail");
    let image = build_x86_image(FAIL_MOD, &dir, "fail");

    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = tyu::runner::Runner::Qemu(runner)
        .run(&image, Duration::from_secs(10))
        .expect("QEMU run");

    let summary = harness_core::parse_output(&outcome.stdout);
    assert!(summary.failures > 0, "F marker should be detected");
}

#[test]
fn run_qemu_no_completion() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }

    let dir = temp_dir("nocomplete");
    let image = build_x86_image(NO_COMPLETION_MOD, &dir, "nocomplete");

    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let outcome = tyu::runner::Runner::Qemu(runner)
        .run(&image, Duration::from_secs(10))
        .expect("QEMU run");

    let summary = harness_core::parse_output(&outcome.stdout);
    assert!(!summary.completed, "S\\n should be absent");
}

#[test]
fn run_qemu_hang_detected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }

    let dir = temp_dir("hang");
    let image = build_x86_image(HANG_MOD, &dir, "hang");

    let runner = Target::X86_64UnknownNone.spec().qemu.unwrap();
    // Very short timeout to catch the hang.
    let outcome = tyu::runner::Runner::Qemu(runner)
        .run(&image, Duration::from_millis(500))
        .expect("QEMU run");

    assert!(outcome.timed_out, "hanging image should time out");
}
