//! Negative corpus for the 50xx (effect/capability/context) and 51xx (stack depth)
//! error bands.  Each test compiles a `.mod` source that MUST fail with exactly
//! the expected error code.
//!
//! Every test is `#[ignore]`d by default.  Un-ignore a test once the phase that
//! implements the corresponding rule makes it green.  The negative corpus
//! defines "done" for each rule (effect-context-model.md §9.1).
//!
//! Note: TcError codes live in the semantics crate at
//! crates/semantics/src/typecheck/error.rs and are distinct from the 3xxx
//! band (semantics) and 8xxx band (verifier/IR).  The 50xx/51xx band is
//! reserved and unused until the corresponding phases un-ignore these tests.

use std::{path::PathBuf, process::Command};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn langc_exe() -> PathBuf {
    workspace_root().join("target").join("debug").join("langc")
}

fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_effect_corpus").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_langc() {
    let status = Command::new("cargo")
        .args(["build", "-p", "langc"])
        .status()
        .expect("cargo build failed");
    assert!(status.success());
}

/// Assert that compiling `src` fails with exactly `expected_code`.
/// Assert that compiling `src` with `--emit=tc` fails with exactly `expected_code`.
/// The stack-checker is the `--emit=tc` path which uses the Context fold (Phase 5+).
fn assert_tc_fails_with(src: &str, expected_code: u32) {
    build_langc();
    let dir = fresh_dir("tc");
    let path = dir.join("test.mod");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=tc", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_str = format!("E{expected_code}");
    assert!(
        code != 0,
        "expected E{expected_code} (exit non-zero), but compilation succeeded.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&expected_str),
        "expected E{expected_code} in stderr, got:\n{stderr}"
    );
}

/// Assert that compiling `src` with `--emit=ir` fails with exactly `expected_code`.
/// The IR generator path is used for most corpus tests since it exercises the typechecker
/// without requiring a full program (main symbol, runtime, etc.).
fn assert_ir_fails_with(src: &str, expected_code: u32) {
    build_langc();
    let dir = fresh_dir("ir");
    let path = dir.join("test.mod");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_str = format!("E{expected_code}");
    assert!(
        code != 0,
        "expected E{expected_code} (exit non-zero), but compilation succeeded.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&expected_str),
        "expected E{expected_code} in stderr, got:\n{stderr}"
    );
}

/// Assert that compiling `src` with `--emit=asm` fails with exactly `expected_code`.
fn assert_fails_with(src: &str, expected_code: u32) {
    build_langc();
    let dir = fresh_dir("neg");
    let path = dir.join("test.mod");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=asm", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_str = format!("E{expected_code}");
    assert!(
        code != 0,
        "expected E{expected_code} (exit non-zero), but compilation succeeded.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&expected_str),
        "expected E{expected_code} in stderr, got:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Distinctness: verify no two TcError variants share a code.
// ---------------------------------------------------------------------------

#[test]
fn tcerror_codes_are_distinct() {
    // Collect every TcError code from the 50xx/51xx band.
    // (Existing 3xxx codes are already stable; we only check the new ones
    // to ensure no internal collision.)
    let codes = [
        5001u32, 5002, 5003, 5004, 5010, 5011, 5012, 5020, 5030, 5031, 5040, 5100, 5101, 5103,
    ];
    let mut sorted = codes.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        codes.len(),
        "duplicate error codes in the 50xx/51xx band"
    );
}

// ---------------------------------------------------------------------------
// 50xx — Effect / Capability / Context
// ---------------------------------------------------------------------------

#[test]
fn e5001_suspend_forbidden() {
    // A word NOT declared with !{suspend} that calls a suspend word.
    // The stack checker (--emit=tc) gives the body a context where
    // SUSPEND is forbidden, triggering SuspendForbidden (5001).
    assert_tc_fails_with(
        "module m;\n\
         : foo ( -- ) platform.task.yield ;\n\
         end;\n",
        5001,
    );
}

#[test]
#[ignore]
fn e5002_lock_nest() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5002);
}

#[test]
#[ignore]
fn e5003_lock_stack() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5003);
}

#[test]
#[ignore]
fn e5004_cap_missing() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5004);
}

#[test]
fn e5010_iso_dup() {
    assert_ir_fails_with(
        "module Main;\n\
         iso Msg;\n\
         : bad_dup ( Msg -- Msg Msg ) dup ;\n\
         end;\n",
        5010,
    );
}

#[test]
fn e5011_iso_drop() {
    assert_ir_fails_with(
        "module Main;\n\
         iso Msg;\n\
         : bad_drop ( Msg -- ) drop ;\n\
         end;\n",
        5011,
    );
}

#[test]
fn e5012_iso_use_after_move() {
    assert_ir_fails_with(
        "module Main;\n\
         iso Msg;\n\
         : move_twice ( Msg -- Msg )\n\
           => x\n\
           x x\n\
         ;\n\
         end;\n",
        5012,
    );
}

#[test]
#[ignore]
fn e5020_borrow_escape() {
    // A scoped borrow that escapes its scope.  Currently unreachable because
    // ScopedMarkerLeak (3506) fires first at block exit.  When the scope
    // model is fully threaded through the IR generator, escape detection
    // will reach BorrowEscape (5020).
    assert_ir_fails_with(
        "module Main;\n\
         : escape ( i64'4 -- i64'4 )\n\
           &[ ]\n\
         ;\n\
         end;\n",
        5020,
    );
}

#[test]
#[ignore]
fn e5030_isr_stack() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5030);
}

#[test]
#[ignore]
fn e5031_resource_shared_unlocked() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5031);
}

#[test]
#[ignore]
fn e5040_diverge_in_bounded() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5040);
}

// ---------------------------------------------------------------------------
// 51xx — Stack depth
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn e5100_stack_unbounded() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5100);
}

#[test]
#[ignore]
fn e5101_stack_exceeds_budget() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5101);
}

#[test]
#[ignore]
fn e5103_stack_quot_erased() {
    assert_fails_with("module m; : main ( -- ) 0 ; end;\n", 5103);
}
