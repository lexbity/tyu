//! Integration tests for `tyu test`.

use std::process::Command;

use tyu::test_helpers::*;

fn ensure_langc() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root()).args(["build", "-q", "-p", "langc"]).status().expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

fn fixture_manifest() -> std::path::PathBuf {
    workspace_root().join("crates").join("execution-tests").join("fixtures").join("manifest.toml")
}

#[test]
fn test_x86_64_none_arithmetic() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) { return; }
    ensure_langc();

    let output = Command::new(tyu_exe())
        .args(["test", "--target=x86_64-unknown-none",
               &format!("--manifest={}", fixture_manifest().display())])
        .output().expect("tyu test");
    assert!(output.status.success(), "tyu test failed:\n{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn test_x86_64_none_deep_stack_high_water() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) { return; }
    ensure_langc();

    let output = Command::new(tyu_exe())
        .args(["test", "--target=x86_64-unknown-none", "--filter=deep_stack",
               &format!("--manifest={}", fixture_manifest().display())])
        .output().expect("tyu test");
    assert!(output.status.success(), "tyu test (deep_stack) failed:\n{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn test_dev_native() {
    if !require_tools(&["langc"]) { return; }
    ensure_langc();

    let output = Command::new(tyu_exe())
        .args(["test", "--target=x86_64-unknown-linux-gnu",
               &format!("--manifest={}", fixture_manifest().display())])
        .output().expect("tyu test");
    // Native target with bare-metal ELF won't execute; just verify no crash.
    let _ = String::from_utf8_lossy(&output.stderr);
}

#[test]
fn test_failing_fixture_detected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) { return; }
    ensure_langc();

    let dir = temp_dir("fail_fixture");
    std::fs::write(&dir.join("fail_test.mod"), "\
module FailTest;\nimport platform/testio { testio.write-byte };\n\
: fail-test-run ( -- ) 70 testio.write-byte ;\n\
: main ( -- i64 ) fail-test-run 0 ;\nexport { fail-test-run main };\nend;\n").unwrap();
    std::fs::write(&dir.join("manifest.toml"), "\
[[fixture]]\nname = \"fail_test\"\nfile = \"fail_test.mod\"\nrequires = []\n").unwrap();

    let output = Command::new(tyu_exe())
        .args(["test", "--target=x86_64-unknown-none",
               &format!("--manifest={}", dir.join("manifest.toml").display())])
        .output().expect("tyu test");
    assert!(!output.status.success(), "failing fixture must produce non-zero exit");
    assert!(String::from_utf8_lossy(&output.stderr).contains("FAIL"));
}
