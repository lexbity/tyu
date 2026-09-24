//! Diagnostic-order tests for the `--emit=obj` driver path (static-verification.md
//! slice P1): the entry gate (E7001 missing-main, E1018 main arity) runs AFTER
//! the typecheck pass, so a semantic error is reported as itself instead of
//! masking as "missing word: main". The handler-only fixture below has no
//! `main` and an `@interrupt` word peaking above the N_isr budget (32 slots);
//! the driver must report E5030, never E7001.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Handler-only module: no `main`, ISR word with a peak of 34 slots (> 32).
const HANDLER_ONLY_NO_MAIN: &str = "\
module Main;
@interrupt(TIMER0) : isr ( -- )
  0
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop
;
end;
";

/// Plain module without `main` but no semantic errors: the entry gate must
/// still reject it with E7001 after the typecheck passes.
const NO_MAIN_CLEAN: &str = "\
module Main;
: helper ( -- i64 )
  0
;
end;
";

/// Module whose `main` returns zero values: E1018 must still fire after the
/// typecheck passes.
const MAIN_BAD_ARITY: &str = "\
module Main;
: main ( -- )
  0 drop
;
end;
";

/// Module whose `main` returns exactly one value: the happy path must be
/// unchanged by the gate reorder.
const VALID_EXE: &str = "\
module Main;
: main ( -- i64 )
  0
;
end;
";

fn langc_exe() -> PathBuf {
    // CARGO_BIN_EXE_ is set for integration tests of the same package, so the
    // binary is guaranteed to be built and the path is target-dir-correct.
    PathBuf::from(env!("CARGO_BIN_EXE_langc"))
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-langc-diag-order-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Compile `source` with `--emit=obj`; return (exit_code, stderr).
fn emit_obj(tag: &str, source: &str) -> (i32, String) {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("Main.mod");
    fs::write(&mod_path, source).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let output = Command::new(langc_exe())
        .arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .expect("langc invocation");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = fs::remove_dir_all(&dir);
    (output.status.code().unwrap_or(-1), stderr)
}

#[test]
fn isr_budget_error_wins_over_missing_main() {
    let (code, stderr) = emit_obj("isr-no-main", HANDLER_ONLY_NO_MAIN);
    assert_eq!(code, 2, "compile must fail; stderr: {stderr}");
    assert!(
        stderr.contains("error[E5030]"),
        "expected E5030 (ISR stack), got: {stderr}"
    );
    assert!(
        !stderr.contains("error[E7001]"),
        "E7001 must not mask the semantic error, got: {stderr}"
    );
}

#[test]
fn missing_main_still_rejected_after_clean_typecheck() {
    let (code, stderr) = emit_obj("no-main-clean", NO_MAIN_CLEAN);
    assert_eq!(code, 2, "compile must fail; stderr: {stderr}");
    assert!(
        stderr.contains("error[E7001]"),
        "expected E7001 (missing main), got: {stderr}"
    );
}

#[test]
fn main_arity_error_still_rejected_after_clean_typecheck() {
    let (code, stderr) = emit_obj("main-arity", MAIN_BAD_ARITY);
    assert_eq!(code, 2, "compile must fail; stderr: {stderr}");
    assert!(
        stderr.contains("error[E1018]"),
        "expected E1018 (main arity), got: {stderr}"
    );
}

#[test]
fn valid_executable_obj_path_unaffected() {
    let (code, stderr) = emit_obj("valid-exe", VALID_EXE);
    assert_eq!(code, 0, "compile must succeed; stderr: {stderr}");
    assert!(!stderr.contains("error[E"), "no diagnostics: {stderr}");
}
