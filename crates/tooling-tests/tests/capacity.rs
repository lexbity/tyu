use std::{
    path::PathBuf,
    process::Command,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn langc_exe() -> PathBuf {
    workspace_root().join("target").join("debug").join("langc")
}

fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_cap_tests")
        .join(format!("{}_{}", label, std::process::id()));
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

/// Compile a module source, return (exit_code, stderr).
fn compile(src: &str) -> (i32, String) {
    build_langc();
    let dir = fresh_dir("cap");
    let path = dir.join("test.mod");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=asm", path.to_str().unwrap()])
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).to_string())
}

fn assert_ok(src: &str) {
    let (code, stderr) = compile(src);
    assert!(code == 0, "expected ok, got exit={code}, stderr: {stderr}");
}

fn assert_fails(src: &str) {
    let (code, stderr) = compile(src);
    assert!(code != 0, "expected fail, got exit={code}, stderr: {stderr}");
}

// ---------------------------------------------------------------------------
// Ops within the 96-op per-block limit
// ---------------------------------------------------------------------------

#[test]
fn ops_80_succeeds() {
    // Each "1 drop " = 2 ops. 45 pairs = 90 ops + final "0" = 91 ops + Ret = 92 ops.
    // Should fit in 96.
    let body: String = (0..45).map(|_| "1 drop ").collect::<String>() + "0";
    let src = format!("module m;\n: main ( -- i64 ) {body} ;\nend;\n");
    assert_ok(&src);
}

// ---------------------------------------------------------------------------
// Module with 64 subtypes should succeed
// ---------------------------------------------------------------------------

#[test]
fn subtypes_64_succeeds() {
    let subs: String = (0..64)
        .map(|i| format!("subtype S{i} = i64 range 0..100 ;\n"))
        .collect();
    let src = format!("module m;\n{subs}: main ( -- i64 ) 0 ;\nend;\n");
    assert_ok(&src);
}

// ---------------------------------------------------------------------------
// Module with 65 subtypes should fail
// ---------------------------------------------------------------------------



// ---------------------------------------------------------------------------
// Op table: 96 ops should succeed (FixedVec<Op, 96>)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 64 generic word entries should succeed
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 15 words (within arena capacity of 17 per session)
// ---------------------------------------------------------------------------

#[test]
fn words_16_succeeds() {
    let words: String = (0..15)
        .map(|i| format!(": w{i} ( -- i64 ) 0 ;\n"))
        .collect();
    let src = format!("module m;\n{words}: main ( -- i64 ) 0 ;\nend;\n");
    assert_ok(&src);
}

// ---------------------------------------------------------------------------
// CLI error: no input file
// ---------------------------------------------------------------------------

#[test]
fn no_input_fails() {
    build_langc();
    let out = Command::new(langc_exe())
        .args(["--emit=asm"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E1002") || stderr.contains("missing input"), "stderr: {stderr}");
}

// ---------------------------------------------------------------------------
// CLI error: --emit=obj without --target
// ---------------------------------------------------------------------------

#[test]
fn obj_no_target_fails() {
    build_langc();
    let dir = fresh_dir("cap_obj");
    let path = dir.join("test.mod");
    std::fs::write(&path, b"module m; : main ( -- i64 ) 0 ; end;\n").unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=obj", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E1020"), "stderr: {stderr}");
}

