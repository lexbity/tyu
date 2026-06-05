//! Tests for `tyu run` on native targets.

use std::process::Command;
use std::time::Duration;

use tyu::runner::{RunOutcome, Runner};
use tyu::test_helpers::*;

const MINIMAL_MAIN: &str = "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
";

fn ensure_langc() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root()).args(["build", "-q", "-p", "langc"]).status().expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

#[test]
fn run_minimal_hosted() {
    if !require_tools(&["langc"]) { return; }
    ensure_langc();

    let dir = temp_dir("hosted_run");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, MINIMAL_MAIN).unwrap();
    let out_dir = dir.join("out");

    let build_out = Command::new(tyu_exe())
        .args(["build", &format!("--out-dir={}", out_dir.display()), &main_mod.to_string_lossy()])
        .output().expect("tyu build");
    assert!(build_out.status.success());

    let image = out_dir.join("image.elf");
    let outcome = Runner::Native.run(&image, Duration::from_secs(5)).unwrap();
    assert!(!outcome.timed_out);
    assert_eq!(outcome.exit_code, 0);
}

#[test]
fn run_exit_code_captured() {
    if !require_tools(&["langc"]) { return; }
    ensure_langc();

    let dir = temp_dir("exit_code");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, "\
module Main;\n: main ( -- i64 ) 42 ;\nexport { main };\nend;\n").unwrap();
    let out_dir = dir.join("out");

    let _ = Command::new(tyu_exe())
        .args(["build", &format!("--out-dir={}", out_dir.display()), &main_mod.to_string_lossy()])
        .output();

    let image = out_dir.join("image.elf");
    let outcome = Runner::Native.run(&image, Duration::from_secs(5)).unwrap();
    assert_eq!(outcome.exit_code, 42);
}

#[test]
fn run_hang_is_timed_out() {
    if !require_tools(&["langc"]) { return; }
    ensure_langc();

    let dir = temp_dir("hang");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, "\
module Main;\n: main ( -- i64 ) main ;\nexport { main };\nend;\n").unwrap();
    let out_dir = dir.join("out");

    let _ = Command::new(tyu_exe())
        .args(["build", &format!("--out-dir={}", out_dir.display()), &main_mod.to_string_lossy()])
        .output();

    let image = out_dir.join("image.elf");
    let outcome = Runner::Native.run(&image, Duration::from_millis(500)).unwrap();
    assert!(outcome.timed_out);
}
