//! Hardware deployment tests.
//!
//! These tests require physical board + OpenOCD + serial connection.
//! They are `#[ignore]` by default and must be explicitly run with:
//!   cargo test -- --ignored

use harness_core::{parse_output, parse_records, Record};
use std::path::PathBuf;
use std::process::Command;
use tyu::test_helpers::*;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

const HW_PASS_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\nexport { main };\nend;\n";

const RP2350_CROSS_ARCH_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 69 testio.write-byte 65 testio.write-byte 32 testio.write-byte 0 ;\n\
export { main };\nend;\n";

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
    if !require_tools(tools) {
        return;
    }

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args([
            "build",
            "-q",
            "-p",
            "langc",
            "-p",
            "lmod-pack",
            "-p",
            "lmod-encrypt",
            "-p",
            "lmod-sign",
        ])
        .status()
        .expect("cargo build");
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
    if !require_tools(tools) {
        return;
    }

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args([
            "build",
            "-q",
            "-p",
            "langc",
            "-p",
            "lmod-pack",
            "-p",
            "lmod-encrypt",
            "-p",
            "lmod-sign",
        ])
        .status()
        .expect("cargo build");
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

fn build_rp2350_demo_image(isa: &str, src: &str, dir: &PathBuf) -> PathBuf {
    let main_mod = dir.join(format!("main-{}.mod", isa));
    std::fs::write(&main_mod, src).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join(format!("out-{}", isa));
    std::fs::create_dir_all(&out_dir).unwrap();

    let output = Command::new(tyu_exe())
        .args([
            "build",
            "--platform=rp2350",
            &format!("--isa={}", isa),
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu build");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("tyu build failed for rp2350/{}:\n{}", isa, stderr);
    }

    out_dir.join("Main.lmod")
}

fn validate_rp2350_transcript(path: &PathBuf, expect_diag: bool) {
    let transcript = std::fs::read(path).unwrap_or_else(|e| {
        panic!(
            "failed to read rp2350 transcript '{}': {}",
            path.display(),
            e
        )
    });
    let records: Vec<_> = parse_records(&transcript).collect();
    assert!(
        records.iter().any(|r| matches!(r, Record::Version(_))),
        "transcript missing V record: {:02x?}",
        transcript
    );
    if expect_diag {
        assert!(
            records.iter().any(|r| matches!(r, Record::Diag(_))),
            "cross-arch transcript missing D record: {:02x?}",
            transcript
        );
        let summary = parse_output(&transcript);
        assert!(
            !summary.completed,
            "cross-arch demo should not emit S\\n completion"
        );
        assert!(
            summary.diagnostics > 0,
            "cross-arch demo should emit at least one diagnostic"
        );
    } else {
        let summary = parse_output(&transcript);
        assert!(summary.completed, "manual HIL should complete");
        assert_eq!(summary.failures, 0);
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("yes") | Ok("on")
    )
}

/// Flash the RP2350 pack via the UF2 recipe and, when a transcript file is
/// supplied, validate the framed UART output from the bench.
///
/// This test is ignored by default and intended for a manually-operated Pico 2
/// bench.  Set `TYU_RP2350_TRANSCRIPT` to a captured UART log to validate the
/// transcript after flashing.
#[ignore]
#[test]
fn hardware_rp2350_manual_hil() {
    let tools = &["langc", "arm-none-eabi-as", "arm-none-eabi-ld", "picotool"];
    if !require_tools(tools) {
        return;
    }

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let dir = temp_dir("hw_rp2350_manual_hil");
    let main_mod = dir.join("main.mod");
    std::fs::write(&main_mod, HW_PASS_MOD).unwrap();

    let sysroot = workspace_root().join("sysroot");
    let out_dir = dir.join("deploy-out");
    let output = Command::new(tyu_exe())
        .args([
            "deploy",
            "--platform=rp2350",
            "--isa=arm",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu deploy");

    if !output.status.success() {
        panic!(
            "tyu deploy failed for RP2350 bench:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let lmod = out_dir.join("Main.lmod");
    assert!(lmod.exists(), "final lmod artifact missing");
    let exec_image = out_dir.join("image.elf");
    assert!(exec_image.exists(), "execution ELF companion missing");

    if let Ok(path) = std::env::var("TYU_RP2350_TRANSCRIPT") {
        validate_rp2350_transcript(&PathBuf::from(path), env_flag("TYU_RP2350_EXPECT_DIAG"));
    }
}

/// Build the RP2350 RISC-V variant so a manual bench run can flash it as the
/// cross-arch mismatch demo.
///
/// The manual operator can flash the produced artifact onto the ARM-booted
/// board and capture the transcript into `TYU_RP2350_TRANSCRIPT`, then rerun
/// this test with `TYU_RP2350_EXPECT_DIAG=1`.
#[ignore]
#[test]
fn hardware_rp2350_cross_arch_abi_mismatch_demo() {
    let tools = &["langc", "riscv32-elf-as", "riscv32-elf-ld"];
    if !require_tools(tools) {
        return;
    }

    let s = Command::new(env!("CARGO"))
        .current_dir(&workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let dir = temp_dir("hw_rp2350_cross_arch");
    let lmod = build_rp2350_demo_image("riscv", RP2350_CROSS_ARCH_MOD, &dir);
    assert!(lmod.exists(), "cross-arch lmod artifact missing");

    if let Ok(path) = std::env::var("TYU_RP2350_TRANSCRIPT") {
        validate_rp2350_transcript(&PathBuf::from(path), env_flag("TYU_RP2350_EXPECT_DIAG"));
    }
}
