//! Regression tests for `tyu` error handling.
//!
//! B8c: Confirms that `tyu` returns a clean non-zero exit code on recoverable
//! errors, rather than aborting via SIGABRT.  Under `panic = "abort"` a panic
//! manifests as signal 6 (SIGABRT); a `?`-propagated `Err` manifests as a
//! normal exit with code 1.

use std::process::Command;

use tyu::test_helpers::*;

#[test]
fn missing_input_file_returns_exit_code_1() {
    let dir = temp_dir("err_missing_input");
    let out_dir = dir.join("out");

    let output = Command::new(tyu_exe())
        .args([
            "build",
            "--target=x86_64-unknown-none",
            &format!("--out-dir={}", out_dir.display()),
            dir.join("nonexistent.mod").to_string_lossy().as_ref(),
        ])
        .output()
        .expect("tyu build");
    assert!(
        !output.status.success(),
        "build with missing input must fail"
    );
    // A clean error exit is code 1.  A panic under abort would
    // produce signal 6 (SIGABRT), which would be caught as a non-exit-code
    // status (status.signal() == 6 on Unix).
    assert_eq!(
        output.status.code(),
        Some(1),
        "exit code must be 1 (clean error), not an abort signal"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error") || stderr.contains("not found"),
        "stderr must contain a diagnostic message, got: {}",
        stderr
    );
}

#[test]
fn missing_key_for_encrypt_returns_exit_code_1() {
    let dir = temp_dir("err_missing_key");
    let main_mod = dir.join("Main.mod");
    std::fs::write(
        &main_mod,
        "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
",
    )
    .unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    // Build first so the .o exists.
    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success());

    // Deploy with --encrypt=fleet but no --key-encrypt.
    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=fleet",
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");
    assert!(
        !output.status.success(),
        "deploy with --encrypt=fleet but no key must fail"
    );
    let code = output.status.code();
    assert_eq!(
        code,
        Some(1),
        "exit code must be 1 (clean error), got: {:?}",
        code
    );
}

// ---------------------------------------------------------------------------
// CLI argument-parsing exit codes.
//
// Regression for the silent-success bug: `tyu test` (and every subcommand)
// previously returned `Command::Help` on an unrecognized flag, which `main`
// dispatched to exit 0 — so a scripted invocation with a typo'd or unsupported
// flag "passed". Usage errors now exit 2; explicit help still exits 0.
// ---------------------------------------------------------------------------

#[test]
fn unknown_test_flag_exits_2_not_0() {
    let output = Command::new(tyu_exe())
        .args(["test", "--definitely-not-a-flag"])
        .output()
        .expect("tyu test");
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unknown `tyu test` flag must be a usage error (exit 2), not a silent success"
    );
}

#[test]
fn unknown_subcommand_exits_2() {
    let output = Command::new(tyu_exe())
        .args(["frobnicate"])
        .output()
        .expect("tyu");
    assert_eq!(output.status.code(), Some(2), "unknown command must exit 2");
}

#[test]
fn help_flag_exits_0() {
    let output = Command::new(tyu_exe())
        .args(["--help"])
        .output()
        .expect("tyu --help");
    assert_eq!(output.status.code(), Some(0), "explicit --help must exit 0");
}

#[test]
fn test_invalid_mode_exits_2() {
    let output = Command::new(tyu_exe())
        .args(["test", "--mode=sideways"])
        .output()
        .expect("tyu test");
    assert_eq!(
        output.status.code(),
        Some(2),
        "an invalid --mode value must be a usage error (exit 2)"
    );
}

#[test]
fn test_mode_dynamic_is_gated_with_clear_message() {
    // Dynamic mode is intentionally rejected by the suite runner (single-module
    // modpack vs. two-module fixture+runner harness). It must fail loudly with a
    // pointer to the cargo dynamic suites — never silently fall back to static.
    let output = Command::new(tyu_exe())
        .args(["test", "--mode=dynamic", "--target=x86_64-unknown-none"])
        .output()
        .expect("tyu test");
    assert_eq!(
        output.status.code(),
        Some(1),
        "--mode=dynamic must be a clean runtime error (exit 1), not exit 0"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--mode=dynamic is not yet supported"),
        "stderr must explain why dynamic is gated, got: {}",
        stderr
    );
}
