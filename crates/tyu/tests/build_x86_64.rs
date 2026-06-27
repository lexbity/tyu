//! End-to-end build test for x86_64-unknown-none.
//!
//! B-1: basic `.lmod` structure.
//! B-2: cache-hit detection via stderr "cache hit" marker.

use std::process::Command;

use lmod::validate::Container;
use tyu::test_helpers::*;

const MINIMAL_MAIN: &str = "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
";

fn build_image(src: &str, dir_label: &str) -> std::path::PathBuf {
    let dir = temp_dir(dir_label);
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, src).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let output = Command::new(tyu_exe())
        .args([
            "build",
            "--mode=static",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu build");

    if !output.status.success() {
        panic!(
            "tyu build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    out_dir.join("Main.lmod")
}

fn build_out_dir(src: &str, dir_label: &str) -> std::path::PathBuf {
    let image = build_image(src, dir_label);
    image.parent().unwrap().to_path_buf()
}

// ---------------------------------------------------------------------------
// B-1: Basic .lmod structure
// ---------------------------------------------------------------------------

#[test]
fn build_minimal_x86_64_none_lmod() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }

    let image = build_image(MINIMAL_MAIN, "minimal_x86_64");
    assert!(image.exists());
    let data = std::fs::read(&image).unwrap();
    let container = Container::parse(&data).unwrap();
    assert_eq!(container.header().format_ver, lmod::header::FORMAT_VER);
    assert!(!container.code().is_empty(), ".lmod must carry code");
}

#[test]
fn build_generates_runtime_symtab_sidecar() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }

    let out_dir = build_out_dir(MINIMAL_MAIN, "runtime_symtab_x86_64");
    let names = std::fs::read_to_string(out_dir.join("lang_symtab.names")).unwrap();
    assert!(
        names.contains("accb676a903a06d9 w_accb676a903a06d9"),
        "runtime symtab must use the canonical platform-word hash"
    );
    assert!(
        names.contains("__lang_ds_high"),
        "runtime symtab must include __lang_* runtime symbols"
    );
    assert!(
        names.contains("__stack_overflow"),
        "runtime symtab must include stack overflow trap"
    );
}

// ---------------------------------------------------------------------------
// B-2: Cache hit detection
// ---------------------------------------------------------------------------

#[test]
fn build_cache_skips_rebuild() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }

    let dir = temp_dir("cached_build");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, MINIMAL_MAIN).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");

    // First build — populates the cache.
    let first = Command::new(tyu_exe())
        .args([
            "build",
            "--mode=static",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("first tyu build");
    assert!(first.status.success());

    // Second build with identical source — should hit cache.
    let second = Command::new(tyu_exe())
        .args([
            "build",
            "--mode=static",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("second tyu build");
    assert!(second.status.success());

    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("cache hit"),
        "B-2: second build with unchanged source must show 'cache hit' on stderr, got: {}",
        stderr
    );

    assert!(out_dir.join("Main.lmod").exists());
}
