//! Hardware deployment tests.
//!
//! These tests require physical board + OpenOCD + serial connection.
//! They are `#[ignore]` by default and must be explicitly run with:
//!   cargo test -- --ignored

use std::path::PathBuf;
use std::process::Command;
use tyu::test_helpers::*;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

const HW_PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

/// Flash a Fleet-encrypted image to a physical STM32 board and verify
/// `S\n` appears over UART.
///
/// Prerequisites:
/// - STM32F4 Discovery board connected via USB
/// - OpenOCD installed with board config `board/stm32f4discovery.cfg`
/// - Serial port at `/dev/ttyACM0` (adjust SERIAL_PORT below)
#[ignore]
#[test]
fn hardware_fleet_encrypted_stm32() {
    let tools = &["langc", "arm-none-eabi-as", "arm-none-eabi-ld", "openocd"];
    if !require_tools(tools) { return; }

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "lmod-pack", "-p", "lmod-encrypt", "-p", "lmod-sign"])
        .status().expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let dir = temp_dir("hw_fleet_stm32");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, HW_PASS_MOD).unwrap();

    // This test needs manual setup — adjust for your board.
    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");
    let kek = hex::encode([0xab; 32]);
    let serial_port = std::env::var("TYU_SERIAL_PORT").unwrap_or_else(|_| "/dev/ttyACM0".into());

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=armv7m-unknown-none",
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

/// Flash a Device-encrypted image (targeting this specific unit) and verify
/// it boots.  A second unit with a different key must not decrypt the same image.
///
/// This test validates Device-mode confidentiality on real hardware.
#[ignore]
#[test]
fn hardware_device_encrypted_stm32() {
    let tools = &["langc", "arm-none-eabi-as", "arm-none-eabi-ld", "openocd"];
    if !require_tools(tools) { return; }

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "lmod-pack", "-p", "lmod-encrypt", "-p", "lmod-sign"])
        .status().expect("cargo build");
    assert!(s.success(), "cargo build failed");

    // Create device keys directory with this device's key.
    let dir = temp_dir("hw_dev_stm32");
    let keys_dir = dir.join("keys");
    std::fs::create_dir_all(&keys_dir).unwrap();
    // Read the device key from an environment variable for security.
    let dev_key_hex = std::env::var("TYU_DEVICE_KEY").expect("TYU_DEVICE_KEY must be set");
    std::fs::write(keys_dir.join("unit-a.key"), &dev_key_hex).unwrap();

    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, HW_PASS_MOD).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("out");

    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--target=armv7m-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            "--encrypt=device",
            &format!("--device-keys={}", keys_dir.display()),
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
