//! D-15: Float literal reservation tests.
//!
//! Float literals are reserved for future syntax.  These tests verify that
//! the compiler rejects:
//!   - Number-dot-number in term position (`3.5`) → E5050
//!   - `e`/`E` exponent in non-hex numbers (`1e5`) → E5051
//!
//! And accepts:
//!   - Hex digit `e` in `0x`-prefixed numbers (`0x1e5`)
//!   - Place-projection `.N` on identifiers (`m.3.5`-shape, staged as `m'3'5`
//!     for now — migrated by S-14's sweep)

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
    let dir = std::env::temp_dir().join("tyu_float_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
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

// ---------------------------------------------------------------------------
// NEG: number-dot-number in term position → E5050
// ---------------------------------------------------------------------------

#[test]
fn float_dot_number_rejected() {
    let dir = fresh_dir("float_dot_number");
    let src = b"module Main;\n\
: main ( -- i64 )\n\
  3.5 drop\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(err.contains("5050"),
        "expected E5050 for number-dot-number, got: {err}");
}

// ---------------------------------------------------------------------------
// NEG: `e`-exponent in non-hex number → E5051
// ---------------------------------------------------------------------------

#[test]
fn float_e_exponent_rejected() {
    let dir = fresh_dir("float_e_exponent");
    let src = b"module Main;\n\
: main ( -- i64 )\n\
  1e5 drop\n\
;\n\
end;\n";
    let err = compile_expect_err(src, &dir);
    assert!(err.contains("5051"),
        "expected E5051 for e-exponent number, got: {err}");
}

// ---------------------------------------------------------------------------
// POS: `0x`-prefixed hex with digit `e` → accepted (not a float exponent)
// ---------------------------------------------------------------------------

#[test]
fn hex_with_e_accepted() {
    let dir = fresh_dir("hex_with_e");
    let src = b"module Main;\n\
: main ( -- i64 )\n\
  0x1e5\n\
;\n\
end;\n";
    compile_ok(src, &dir);
}

// ---------------------------------------------------------------------------
// POS: place-projection `.N` via `'N` (staged for S-14 migration)
// ---------------------------------------------------------------------------

#[test]
fn place_projection_via_apostrophe() {
    let dir = fresh_dir("place_projection_via_apostrophe");
    let src = b"module Main;\n\
register-map GPIO\n\
  0x00 DATA[4] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 )\n\
  gpio.DATA'2 @u32 drop\n\
  0\n\
;\n\
end;\n";
    compile_ok(src, &dir);
}
