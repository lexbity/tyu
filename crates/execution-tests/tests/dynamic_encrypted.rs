//! Signed+encrypted `.lmod` loading under the firmware-resident QEMU loader.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const SIGN_KEY_HEX: &str = "abababababababababababababababababababababababababababababababab";
const KEK_HEX: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
const WRONG_KEK_HEX: &str = "efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef";

const PASS_MOD: &str = "module Main;\n\
import platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\n\
export { main };\nend;\n";

const ENCRYPTED_TARGETS: &[common::DynamicTarget] = &[common::DynamicTarget {
    target: codegen_core::Target::X86_64UnknownNone,
    triple: "x86_64-unknown-none",
    tools: common::X86_DYNAMIC_TOOLS,
}];

fn build_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build langc tyu");
    assert!(status.success(), "cargo build langc tyu failed");
}

fn build_encrypted_dynamic_image(
    target: common::DynamicTarget,
    dir: &Path,
    metal_encrypt_mode: Option<&str>,
    mutation: Option<&str>,
    encrypt_with: Option<&str>,
) -> PathBuf {
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let label = metal_encrypt_mode
        .or(mutation)
        .or(encrypt_with)
        .unwrap_or("positive");
    let out_dir = dir.join(format!("{}_encrypted_{}", target.triple, label).replace('-', "_"));
    let sysroot = common::workspace_root().join("sysroot");

    let mut command = Command::new(common::tyu_exe());
    command
        .current_dir(common::workspace_root())
        .env("TYU_METAL_SIGN_KEY", SIGN_KEY_HEX)
        .env("TYU_METAL_KEK", KEK_HEX)
        .args([
            "build",
            "--mode=dynamic",
            "--metal-sign-key=env:TYU_METAL_SIGN_KEY",
            "--metal-kek=env:TYU_METAL_KEK",
            &format!("--target={}", target.triple),
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ]);
    if let Some(mode) = metal_encrypt_mode {
        command.arg(format!("--metal-encrypt={mode}"));
    }
    if let Some(mutation) = mutation {
        command.env("TYU_TEST_MUTATE_LMOD", mutation);
    }
    if let Some(kek) = encrypt_with {
        command.env("TYU_TEST_ENCRYPT_WITH_KEK", kek);
    }

    let output = command.output().expect("tyu build encrypted dynamic");
    assert!(
        output.status.success(),
        "{} encrypted dynamic build failed:\nstdout:\n{}\nstderr:\n{}",
        target.triple,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    assert!(
        out_dir.join("keys_generated.o").exists(),
        "encrypted dynamic firmware must include generated key object"
    );
    let image = out_dir.join("image.elf");
    assert!(
        image.exists(),
        "dynamic firmware missing: {}",
        image.display()
    );
    image
}

fn trap_codes(stdout: &[u8]) -> Vec<u16> {
    harness_core::parse_records(stdout)
        .filter_map(|record| match record {
            harness_core::Record::Diag(payload) => {
                diag_core::DiagRecord::parse(payload).map(|diag| diag.trap_code)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn encrypted_dynamic_lmod_runs_under_qemu() {
    for target in ENCRYPTED_TARGETS {
        if !common::require_tool_groups(target.tools) {
            return;
        }
        build_tools();
        let dir = common::temp_dir(&format!("dynamic_encrypted_{}", target.triple));
        let image = build_encrypted_dynamic_image(*target, &dir, None, None, None);
        let outcome =
            common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
        assert!(
            !outcome.timed_out,
            "{} encrypted run timed out",
            target.triple
        );
        let parsed = harness_core::parse_output(&outcome.stdout);
        assert!(
            parsed.completed && parsed.failures == 0,
            "{} encrypted load must complete successfully, stdout bytes: {:02x?}",
            target.triple,
            outcome.stdout,
        );
    }
}

#[test]
fn device_encrypted_dynamic_lmod_runs_under_qemu() {
    for target in ENCRYPTED_TARGETS {
        if !common::require_tool_groups(target.tools) {
            return;
        }
        build_tools();
        let dir = common::temp_dir(&format!("dynamic_device_encrypted_{}", target.triple));
        let image = build_encrypted_dynamic_image(*target, &dir, Some("device"), None, None);
        let outcome =
            common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
        assert!(
            !outcome.timed_out,
            "{} device encrypted run timed out",
            target.triple
        );
        let parsed = harness_core::parse_output(&outcome.stdout);
        assert!(
            parsed.completed && parsed.failures == 0,
            "{} device encrypted load must complete successfully, stdout bytes: {:02x?}",
            target.triple,
            outcome.stdout,
        );
    }
}

#[test]
fn encrypted_dynamic_wrong_key_traps_5215() {
    for target in ENCRYPTED_TARGETS {
        if !common::require_tool_groups(target.tools) {
            return;
        }
        build_tools();
        let dir = common::temp_dir(&format!("dynamic_encrypted_wrong_key_{}", target.triple));
        let image = build_encrypted_dynamic_image(*target, &dir, None, None, Some(WRONG_KEK_HEX));
        let outcome =
            common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
        assert!(
            !outcome.timed_out,
            "{} wrong-key run timed out",
            target.triple
        );
        let codes = trap_codes(&outcome.stdout);
        assert!(
            codes.contains(&5215),
            "{} wrong-key expected E_ENC_NO_KEY/5215, got {:?}\nstdout bytes: {:02x?}",
            target.triple,
            codes,
            outcome.stdout,
        );
    }
}

#[test]
fn encrypted_dynamic_tamper_traps_5202() {
    for target in ENCRYPTED_TARGETS {
        if !common::require_tool_groups(target.tools) {
            return;
        }
        build_tools();
        let dir = common::temp_dir(&format!("dynamic_encrypted_tamper_{}", target.triple));
        let image = build_encrypted_dynamic_image(*target, &dir, None, Some("has-isr"), None);
        let outcome =
            common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
        assert!(
            !outcome.timed_out,
            "{} tampered encrypted run timed out",
            target.triple
        );
        let codes = trap_codes(&outcome.stdout);
        assert!(
            codes.contains(&5202),
            "{} tamper expected E_SIG_INVALID/5202, got {:?}\nstdout bytes: {:02x?}",
            target.triple,
            codes,
            outcome.stdout,
        );
    }
}
