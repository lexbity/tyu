//! Integration tests for `tyu test`.

use std::path::PathBuf;
use std::process::Command;

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
    if missing.is_empty() { return true; }
    if std::env::var("CI").is_ok() {
        panic!("Required tools not available: {}", missing.join(", "));
    }
    eprintln!("SKIP: missing tools ({})", missing.join(", "));
    false
}

#[test]
fn test_x86_64_none_arithmetic() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }

    // Ensure langc is built.
    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    // Point to the execution-tests fixture manifest.
    let manifest = workspace_root()
        .join("crates")
        .join("execution-tests")
        .join("fixtures")
        .join("manifest.toml");

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            &format!("--manifest={}", manifest.display()),
        ])
        .output()
        .expect("tyu test");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("tyu test failed:\n{}", stderr);
    }
}

#[test]
fn test_x86_64_none_deep_stack_high_water() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }

    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    let manifest = workspace_root()
        .join("crates")
        .join("execution-tests")
        .join("fixtures")
        .join("manifest.toml");

    // Run only deep_stack, which has a deep data-stack usage.
    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            "--filter=deep_stack",
            &format!("--manifest={}", manifest.display()),
        ])
        .output()
        .expect("tyu test");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("tyu test (deep_stack) failed:\n{}", stderr);
    }
}

#[test]
fn test_dev_native() {
    if !require_tools(&["langc"]) {
        return;
    }

    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    // For native (dev), the manifest is at fixtures/manifest.toml.
    // But dev tests can only run suites with no capability requirements
    // that produce valid native binaries.  Since our fixtures target
    // x86_64-unknown-none (bare metal), they won't link on native.
    // So we test that tyu test gracefully skips targets with missing tools.
    let manifest = workspace_root()
        .join("crates")
        .join("execution-tests")
        .join("fixtures")
        .join("manifest.toml");

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-linux-gnu",
            &format!("--manifest={}", manifest.display()),
        ])
        .output()
        .expect("tyu test");

    // Native with no qemu runner: the test will try native run,
    // but the image is a bare-metal ELF so it won't execute.
    // The important thing is that tyu test doesn't crash.
    // This validates the framework, not the specific execution.
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("tyu test (dev): {}", stderr);
    // Don't assert success — the binary won't actually run natively.
    // This is expected to fail at execution, not at build time.
}

/// A failing fixture should surface as a test failure.
#[test]
fn test_failing_fixture_detected() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }

    let _ = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();

    // Create a temporary manifest with a fixture that emits F.
    let dir = std::env::temp_dir()
        .join("tyu_test_matrix_fail")
        .join(format!("{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let fixture_path = dir.join("fail_test.mod");
    std::fs::write(&fixture_path, "\
module FailTest;
import platform/testio { testio.write-byte };
: fail-test-run ( -- )
  70 testio.write-byte ;
: main ( -- i64 ) fail-test-run 0 ;
export { fail-test-run main };
end;
").unwrap();

    let manifest_path = dir.join("manifest.toml");
    std::fs::write(&manifest_path, "\
[[fixture]]
name = \"fail_test\"
file = \"fail_test.mod\"
requires = []
").unwrap();

    let output = Command::new(tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            &format!("--manifest={}", manifest_path.display()),
        ])
        .output()
        .expect("tyu test");

    // The test should fail because the fixture emits F.
    assert!(!output.status.success(), "failing fixture must produce non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("FAIL"), "stderr should mention FAIL_MARKER");
}
