//! BUG-010 regression: subtype range bounds near i64::MAX must assemble.
//!
//! `mov r/m64, imm64` has no x86-64 encoding, so constants outside the
//! sign-extended imm32 range used to be written straight to memory and fasm
//! rejected them with "value out of range" (E1017).  Each test here compiles
//! to an object file — the path that used to fail.

use std::path::PathBuf;
use std::process::Command;

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
    let dir = std::env::temp_dir().join("tyu_subtype_bounds").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Compile the module to an object file; panics with stderr on failure.
fn compile_obj(src: &str, label: &str) {
    let dir = fresh_dir(label);
    std::fs::write(dir.join("test.mod"), src).unwrap();
    let out = Command::new(langc_exe())
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-none",
            "--out-dir=.",
            "test.mod",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "langc --emit=obj failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        dir.join("M.o").exists(),
        "--emit=obj must produce M.o (object is named after the module)"
    );
}

/// The exact reproduction from the bug report: an `i64::MAX` upper bound.
#[test]
fn subtype_max_bound_assembles() {
    compile_obj(
        "module M;\n\
         subtype NonNeg = i64 range 0 .. 9223372036854775807;\n\
         : main ( -- i64 ) 8 as? NonNeg drop drop 0 ;\n\
         export { main };\n\
         end;\n",
        "max_bound",
    );
}

/// Boundary just above the sign-extended imm32 range.
#[test]
fn subtype_above_i32_max_assembles() {
    compile_obj(
        "module M;\n\
         subtype T = i64 range 0 .. 2147483648;\n\
         : main ( -- i64 ) 0 as? T drop drop 0 ;\n\
         export { main };\n\
         end;\n",
        "above_i32_max",
    );
}

/// Boundary just below the sign-extended imm32 range (negative side).
#[test]
fn subtype_below_i32_min_assembles() {
    compile_obj(
        "module M;\n\
         subtype T = i64 range -2147483649 .. 5;\n\
         : main ( -- i64 ) 0 as? T drop drop 0 ;\n\
         export { main };\n\
         end;\n",
        "below_i32_min",
    );
}

/// A value at the exact sign-extended imm32 boundary still uses the compact
/// memory-immediate form.
#[test]
fn subtype_at_i32_max_assembles() {
    compile_obj(
        "module M;\n\
         subtype T = i64 range 0 .. 2147483647;\n\
         : main ( -- i64 ) 0 as? T drop drop 0 ;\n\
         export { main };\n\
         end;\n",
        "at_i32_max",
    );
}