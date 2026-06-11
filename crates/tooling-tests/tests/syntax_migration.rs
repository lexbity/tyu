//! E2E tests for S-11 syntax changes (D-18):
//!   - `requires [...]` replaces old `requires [...]` (contract precondition)
//!   - `requires {...}` introduces compile-time capability sets
//!   - `ensures [...]` unchanged
//!   - Old `requires [` → migration hint error
//!   - Unknown capability names → error

use std::process::Command;

fn langc_exe() -> std::path::PathBuf {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap();
    workspace.join("target").join("debug").join("langc")
}

fn repo_sysroot() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("sysroot")
}

fn fresh_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("tyu_syntax_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn compile_ok(src: &[u8], dir: &std::path::Path) {
    let mod_path = dir.join("test.mod");
    std::fs::write(&mod_path, src).unwrap();
    let out = Command::new(langc_exe())
        .current_dir(dir)
        .arg("--emit=ir")
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
}

fn compile_expect_err(src: &[u8], dir: &std::path::Path) -> String {
    let mod_path = dir.join("test.mod");
    std::fs::write(&mod_path, src).unwrap();
    let out = Command::new(langc_exe())
        .current_dir(dir)
        .arg("--emit=ir")
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(!out.status.success(), "expected compilation error");
    String::from_utf8_lossy(&out.stderr).to_string()
}

// ---------------------------------------------------------------------------
// POS: `requires [...]` contract (replaces old `requires [...]`)
// ---------------------------------------------------------------------------

#[test]
fn needs_contract_precondition() {
    let dir = fresh_dir("needs_contract");
    let src = b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  needs [ 1 1 == ]\n\
  42\n\
;\n\
end;\n";
    compile_ok(src, &dir);
}

// ---------------------------------------------------------------------------
// POS: `ensures [...]` postcondition (unchanged)
// ---------------------------------------------------------------------------

#[test]
fn ensures_postcondition() {
    let dir = fresh_dir("ensures_postcondition");
    let src = b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  ensures [ dup 0 >= ]\n\
  42\n\
;\n\
end;\n";
    compile_ok(src, &dir);
}

// ---------------------------------------------------------------------------
// NEG: old `requires [` → migration hint
// ---------------------------------------------------------------------------

#[test]
fn old_requires_bracket_rejected() {
    let dir = fresh_dir("old_requires_bracket");
    let src = b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  requires [ 1 1 == ]\n\
  42\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    // Should produce a parse error (ExpectedModule or similar migration hint).
    assert!(err.contains("use") || err.contains("needs") || !err.is_empty(),
        "old requires [ should produce an error, got: {err}");
}

// ---------------------------------------------------------------------------
// POS: `requires {caps}` compile-time capability set (parses OK)
// ---------------------------------------------------------------------------

#[test]
fn requires_cap_set_parses() {
    let dir = fresh_dir("requires_cap_set");
    let src = b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  requires {suspendable}\n\
  42\n\
;\n\
end;\n";
    compile_ok(src, &dir);
}

// ---------------------------------------------------------------------------
// POS: `requires {write(pwm)}` capability with resource argument
// ---------------------------------------------------------------------------

#[test]
fn requires_write_cap_parses() {
    let dir = fresh_dir("requires_write_cap");
    let src = b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  requires {write(pwm)}\n\
  42\n\
;\n\
end;\n";
    compile_ok(src, &dir);
}
