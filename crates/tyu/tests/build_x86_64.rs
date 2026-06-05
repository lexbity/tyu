//! End-to-end build test for x86_64-unknown-none.

use std::process::Command;

use tyu::test_helpers::*;

const MINIMAL_MAIN: &str = "\
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
";

#[test]
fn build_minimal_x86_64_none_elf() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }

    let dir = temp_dir("minimal_x86_64");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, MINIMAL_MAIN).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let build_status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status().expect("cargo build");
    assert!(build_status.success());

    let output = Command::new(tyu_exe())
        .args(["build", "--target=x86_64-unknown-none",
               &format!("--sysroot={}", sysroot.display()),
               &format!("--out-dir={}", out_dir.display()),
               &main_mod.to_string_lossy()])
        .output().expect("tyu build");

    if !output.status.success() {
        panic!("tyu build failed:\n{}", String::from_utf8_lossy(&output.stderr));
    }

    let image = out_dir.join("image.elf");
    assert!(image.exists());
    let data = std::fs::read(&image).unwrap();
    assert_eq!(&data[..4], b"\x7fELF");
    assert_eq!(data[4], 2);
}

#[test]
fn build_uses_cache_on_second_run() {
    if !require_tools(&["langc", "fasm", "ld"]) {
        return;
    }

    let dir = temp_dir("cached_build");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, MINIMAL_MAIN).unwrap();
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root()).args(["build", "-q", "-p", "langc"]).status().expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let first = Command::new(tyu_exe())
        .args(["build", "--target=x86_64-unknown-none",
               &format!("--sysroot={}", sysroot.display()),
               &format!("--out-dir={}", out_dir.display()),
               &main_mod.to_string_lossy()])
        .output().expect("first tyu build");
    assert!(first.status.success());

    let second = Command::new(tyu_exe())
        .args(["build", "--target=x86_64-unknown-none",
               &format!("--sysroot={}", sysroot.display()),
               &format!("--out-dir={}", out_dir.display()),
               &main_mod.to_string_lossy()])
        .output().expect("second tyu build");
    assert!(second.status.success());

    assert!(out_dir.join("image.elf").exists());
}
