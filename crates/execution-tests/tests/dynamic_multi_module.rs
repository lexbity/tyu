//! Two-module dynamic images under the firmware-resident QEMU loader
//! (PLAN-VERIFY-3 P11.3 follow-up: the compositional rule's runtime half).
//!
//! The modpack carries the FULL module graph (deps first, each `.lmod`
//! length-prefixed), and `__lang_load_and_run` loads every blob in order,
//! binding each module's imports against the previous modules' exports.
//! This file pins the path end to end: a callee word CALLED from the root's
//! `main` must produce the harness completion marker on every dynamic
//! target (and with per-module signing, since the build signs each embedded
//! `.lmod`).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const SIGN_KEY_HEX: &str = "abababababababababababababababababababababababababababababababab";

const CAL_DEF: &str = "module Cal;\n: cal ( -- i64 ) ;\nexport { cal };\nend;\n";
const CAL_MOD: &str = "module Cal;\n: cal ( -- i64 ) 5 ;\nexport { cal };\nend;\n";
const MAIN_MOD: &str = "\
module Main;\nimport platform/testio { testio.write-byte };\nimport Cal { cal };\n\
: main ( -- i64 ) cal drop 83 testio.write-byte 10 testio.write-byte 0 ;\n\
export { main };\nend;\n";

fn build_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build langc tyu");
    assert!(status.success(), "cargo build langc tyu failed");
}

/// Write the two-module project (`.def` interface + both `.mod` files).
fn write_project(dir: &Path) -> PathBuf {
    std::fs::write(dir.join("Cal.def"), CAL_DEF).unwrap();
    std::fs::write(dir.join("Cal.mod"), CAL_MOD).unwrap();
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, MAIN_MOD).unwrap();
    main_mod
}

fn build_dynamic_image(target: common::DynamicTarget, dir: &Path, signed: bool) -> PathBuf {
    let main_mod = write_project(dir);
    let out_dir = dir.join(
        format!(
            "{}_multi_{}",
            target.triple,
            if signed { "signed" } else { "plain" }
        )
        .replace('-', "_"),
    );
    let sysroot = common::workspace_root().join("sysroot");

    let mut command = Command::new(common::tyu_exe());
    command.current_dir(common::workspace_root()).args([
        "build",
        "--mode=dynamic",
        &format!("--target={}", target.triple),
        &format!("--sysroot={}", sysroot.display()),
        &format!("--out-dir={}", out_dir.display()),
        &main_mod.to_string_lossy(),
    ]);
    if signed {
        command
            .env("TYU_METAL_SIGN_KEY", SIGN_KEY_HEX)
            .arg("--metal-sign-key=env:TYU_METAL_SIGN_KEY");
    }

    let output = command.output().expect("tyu build two-module dynamic");
    assert!(
        output.status.success(),
        "{} two-module dynamic build (signed={signed}) failed:\nstdout:\n{}\nstderr:\n{}",
        target.triple,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let image = out_dir.join("image.elf");
    assert!(
        image.exists(),
        "dynamic firmware missing: {}",
        image.display()
    );
    image
}

fn assert_two_module_runs(target: common::DynamicTarget, signed: bool) {
    if !common::require_tool_groups(target.tools) {
        return;
    }
    build_tools();

    let dir = common::temp_dir(&format!(
        "dynamic_multi_{}_{}",
        target.triple,
        if signed { "signed" } else { "plain" }
    ));
    let image = build_dynamic_image(target, &dir, signed);
    let outcome = common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
    assert!(
        !outcome.timed_out,
        "{} two-module dynamic run (signed={signed}) timed out",
        target.triple
    );
    let parsed = harness_core::parse_output(&outcome.stdout);
    assert!(
        parsed.completed && parsed.failures == 0,
        "{} two-module dynamic image (signed={signed}) must load the callee, \
         bind the import, and complete — stdout bytes: {:02x?}",
        target.triple,
        outcome.stdout,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_module_dynamic_image_completes() {
    for target in common::DYNAMIC_TARGETS {
        assert_two_module_runs(*target, false);
    }
}

/// The signed variant runs on EVERY dynamic target. (It originally excluded
/// ARM: the loader built without size discipline and a signed ARM image
/// overflowed the lm3s6965evb's 256 KiB flash by ~18 KiB. Fixed by building
/// the device loader with fat LTO + 1 CGU (`device-loader-archive`
/// `[profile.release]`) and cutting `lmod → ir → frontend` out of the
/// loader's dependency chain — the signed single-module ARM image is now
/// ~27 KiB of flash, and the signing feature adds ~3 KiB of loader text.)
#[test]
fn two_module_signed_dynamic_image_completes() {
    // The build signs EVERY embedded module; the loader must verify each
    // and still compose the pair.
    for target in common::DYNAMIC_TARGETS {
        assert_two_module_runs(*target, true);
    }
}
