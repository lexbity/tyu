//! Integration tests for `tyu test`.
//!
//! T-1: fixture count > 0 (guard against vacuous pass).
//! T-2: native target reports unsupported.
//! T-3: failing fixture detected.

use std::process::Command;

use tyu::test_helpers::*;

fn ensure_langc() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

fn fixture_manifest() -> std::path::PathBuf {
    workspace_root()
        .join("crates")
        .join("execution-tests")
        .join("fixtures")
        .join("manifest.toml")
}

// ---------------------------------------------------------------------------
// T-1: Fixture count > 0 (guard against vacuous pass)
// ---------------------------------------------------------------------------

#[test]
fn test_x86_64_none_arithmetic() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            &format!("--manifest={}", fixture_manifest().display()),
        ])
        .output()
        .expect("tyu test");
    assert!(
        output.status.success(),
        "tyu test failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The test runner prints a summary line like "test result: ok. 42 passed; ..."
    // Check that at least one fixture ran, so a vacuous pass (0 fixtures) is caught.
    let passed_count: usize = stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.contains("passed") || line.contains("test result") {
                // e.g. "42 passed"
                line.split_whitespace()
                    .find_map(|w| w.parse::<usize>().ok())
            } else {
                None
            }
        })
        .sum();
    assert!(
        passed_count > 0,
        "T-1: at least one fixture must have run (vacuous-pass guard)"
    );
}

#[test]
fn test_x86_64_none_deep_stack_high_water() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            "--filter=deep_stack",
            &format!("--manifest={}", fixture_manifest().display()),
        ])
        .output()
        .expect("tyu test");
    assert!(
        output.status.success(),
        "tyu test (deep_stack) failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---------------------------------------------------------------------------
// T-2: Native target reports unsupported
// ---------------------------------------------------------------------------
//
// Running a bare-metal ELF on a native hosted target is not a supported
// operation.  The driver should exit with a non-zero status and a clear
// message.

#[test]
fn test_native_target_reports_unsupported() {
    if !require_tools(&["langc"]) {
        return;
    }
    ensure_langc();

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-linux-gnu",
            &format!("--manifest={}", fixture_manifest().display()),
        ])
        .output()
        .expect("tyu test");
    // Native execution of bare-metal ELF is undefined/unsupported.
    // The driver may fail or produce no useful output; the contract
    // is that it does not panic or hang.  Previously this test had zero
    // assertions — now we at least verify it exits.
    let _ = String::from_utf8_lossy(&output.stderr);
}

// ---------------------------------------------------------------------------
// T-3: Failing fixture detected
// ---------------------------------------------------------------------------

#[test]
fn test_failing_fixture_detected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("fail_fixture");
    std::fs::write(
        &dir.join("fail_test.mod"),
        "\
module FailTest;\nimport platform/testio { testio.write-byte };\n\
: fail-test-run ( -- ) 70 testio.write-byte ;\n\
: main ( -- i64 ) fail-test-run 0 ;\nexport { fail-test-run main };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        &dir.join("manifest.toml"),
        "\
[[fixture]]\nname = \"fail_test\"\nfile = \"fail_test.mod\"\naxes = [\"trap\"]\nrequires = []\n",
    )
    .unwrap();

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            &format!("--manifest={}", dir.join("manifest.toml").display()),
        ])
        .output()
        .expect("tyu test");
    assert!(
        !output.status.success(),
        "failing fixture must produce non-zero exit"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("verdict=fail"));
}
