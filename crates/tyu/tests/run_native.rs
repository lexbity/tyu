//! Tests for `tyu run` on native targets.
//!
//! R-1: basic pass/exit-code assertions; build-status assertions.
//! R-2: deterministic hang detection.

use std::process::Command;
use std::time::Duration;

use tyu::runner::Runner;
use tyu::test_helpers::*;

const MINIMAL_MAIN: &str = "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
";

fn ensure_langc() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

fn build_hosted(src: &str, dir_label: &str) -> std::path::PathBuf {
    let dir = temp_dir(dir_label);
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, src).unwrap();
    let out_dir = dir.join("out");
    let build_out = Command::new(tyu_exe())
        .args([
            "build",
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu build");
    assert!(
        build_out.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&build_out.stderr)
    );
    out_dir.join("image.elf")
}

// ---------------------------------------------------------------------------
// R-1: Basic pass, exit code, build-status assertions
// ---------------------------------------------------------------------------

#[test]
fn run_minimal_hosted() {
    if !require_tools(&["langc"]) {
        return;
    }
    ensure_langc();

    let image = build_hosted(MINIMAL_MAIN, "hosted_run");
    let outcome = Runner::Native.run(&image, Duration::from_secs(5)).unwrap();
    assert!(!outcome.timed_out);
    assert_eq!(outcome.exit_code, 0);
}

#[test]
fn run_exit_code_captured() {
    if !require_tools(&["langc"]) {
        return;
    }
    ensure_langc();

    // build_hosted already asserts build success.
    let image = build_hosted(
        "\
module Main;\n: main ( -- i64 ) 42 ;\nexport { main };\nend;\n",
        "exit_code",
    );

    let outcome = Runner::Native.run(&image, Duration::from_secs(5)).unwrap();
    assert_eq!(outcome.exit_code, 42);
}

// ---------------------------------------------------------------------------
// R-2: Deterministic hang detection
// ---------------------------------------------------------------------------
//
// Uses an empty quote-loop (`[ ] loop`) that never exits (no recursion,
// no TCO dependency).  The program genuinely does not terminate, so the
// 500 ms timeout expires and timed_out == true.

const HANG_MOD: &str = "\
module Main;\n\
: main ( -- i64 ) 0 [ ] loop drop 0 ;\n\
export { main };\nend;\n";

#[test]
fn run_hang_is_timed_out() {
    if !require_tools(&["langc"]) {
        return;
    }
    ensure_langc();

    let image = build_hosted(HANG_MOD, "hang");
    let outcome = Runner::Native
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
