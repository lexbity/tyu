//! Tests for `tyu deploy` in Fleet mode under QEMU.

use std::process::Command;
use tyu::test_helpers::*;

const PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

fn ensure_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "lmod-pack", "-p", "lmod-encrypt", "-p", "lmod-sign"])
        .status().expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

#[test]
fn deploy_fleet_qemu() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64", "lmod-pack", "lmod-encrypt", "lmod-sign"]) {
        return;
    }
    ensure_tools();

    let dir = temp_dir("deploy_fleet");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    let kek = hex::encode([0xabu8; 32]);

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=x86_64-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=fleet",
            &format!("--key-encrypt={}", kek),
            "--sign",
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("tyu deploy failed:\n{}", stderr);
    }
}
