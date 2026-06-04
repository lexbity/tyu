//! Tests for `tyu run` on native targets.
//!
//! Requires `langc` in PATH.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use tyu::runner::{RunOutcome, Runner};

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
        .join("tyu_run_native_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const MINIMAL_MAIN: &str = "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
";

/// Build a minimal hosted program and run it via `tyu run`.
#[test]
fn run_minimal_hosted() {
    if !require_tools(&["langc"]) {
        return;
    }

    // Ensure langc is built.
    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    let dir = temp_dir("hosted_run");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, MINIMAL_MAIN).unwrap();
    let out_dir = dir.join("out");

    // Build first.
    let tyu = tyu_exe();
    let build_out = Command::new(&tyu)
        .args(["build", &format!("--out-dir={}", out_dir.display()), &main_mod.to_string_lossy()])
        .output()
        .expect("tyu build");
    assert!(build_out.status.success(), "build failed");

    // Run natively via the Runner.
    let image = out_dir.join("image.elf");
    let runner = Runner::Native;
    let outcome = runner.run(&image, Duration::from_secs(5))
        .expect("native run should succeed");

    assert!(!outcome.timed_out, "should not time out");
    assert_eq!(outcome.exit_code, 0, "exit code should be 0");
}

/// Build a program that exits with code 42 and verify the runner captures it.
#[test]
fn run_exit_code_captured() {
    if !require_tools(&["langc"]) {
        return;
    }

    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    let dir = temp_dir("exit_code");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, "\
module Main;
: main ( -- i64 ) 42 ;
export { main };
end;
").unwrap();
    let out_dir = dir.join("out");

    let tyu = tyu_exe();
    let _ = Command::new(&tyu)
        .args(["build", &format!("--out-dir={}", out_dir.display()), &main_mod.to_string_lossy()])
        .output();

    let image = out_dir.join("image.elf");
    let outcome = Runner::Native.run(&image, Duration::from_secs(5))
        .expect("native run");

    assert!(!outcome.timed_out);
    assert_eq!(outcome.exit_code, 42, "exit code must match program return value");
}

/// A program that hangs should be killed after timeout.
#[test]
fn run_hang_is_timed_out() {
    if !require_tools(&["langc"]) {
        return;
    }

    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    let dir = temp_dir("hang");
    // An infinite loop: `begin again` or just a recursive call.
    // Simplest: call main recursively (infinite recursion → hang in practice).
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, "\
module Main;
: main ( -- i64 ) main ;
export { main };
end;
").unwrap();
    let out_dir = dir.join("out");

    let tyu = tyu_exe();
    let _ = Command::new(&tyu)
        .args(["build", &format!("--out-dir={}", out_dir.display()), &main_mod.to_string_lossy()])
        .output();

    let image = out_dir.join("image.elf");
    // Very short timeout to catch the hang.
    let outcome = Runner::Native.run(&image, Duration::from_millis(500))
        .expect("native run");

    assert!(outcome.timed_out, "hanging program should time out");
}
