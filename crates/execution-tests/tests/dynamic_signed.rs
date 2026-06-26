//! Tier-1 signed `.lmod` loading under the firmware-resident QEMU loader.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const SIGN_KEY_HEX: &str = "abababababababababababababababababababababababababababababababab";

const PASS_MOD: &str = "module Main;\n\
import platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\n\
export { main };\nend;\n";

const SIGNED_TARGETS: &[common::DynamicTarget] = &[common::DynamicTarget {
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

fn build_signed_dynamic_image(
    target: common::DynamicTarget,
    dir: &Path,
    mutation: Option<&str>,
) -> PathBuf {
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let label = mutation.unwrap_or("positive");
    let out_dir = dir.join(format!("{}_signed_{}", target.triple, label).replace('-', "_"));
    let sysroot = common::workspace_root().join("sysroot");

    let mut command = Command::new(common::tyu_exe());
    command
        .current_dir(common::workspace_root())
        .env("TYU_METAL_SIGN_KEY", SIGN_KEY_HEX)
        .args([
            "build",
            "--mode=dynamic",
            "--metal-sign-key=env:TYU_METAL_SIGN_KEY",
            &format!("--target={}", target.triple),
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ]);
    if let Some(mutation) = mutation {
        command.env("TYU_TEST_MUTATE_LMOD", mutation);
    }

    let output = command.output().expect("tyu build signed dynamic");
    assert!(
        output.status.success(),
        "{} signed dynamic build failed:\nstdout:\n{}\nstderr:\n{}",
        target.triple,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    assert!(
        out_dir.join("keys_generated.o").exists(),
        "signed dynamic firmware must include generated key object"
    );
    let image = out_dir.join("image.elf");
    assert!(
        image.exists(),
        "dynamic firmware missing: {}",
        image.display()
    );
    image
}

fn assert_signed_runs(target: common::DynamicTarget) {
    if !common::require_tool_groups(target.tools) {
        return;
    }
    build_tools();

    let dir = common::temp_dir(&format!("dynamic_signed_{}", target.triple));
    let image = build_signed_dynamic_image(target, &dir, None);
    let outcome = common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
    assert!(!outcome.timed_out, "{} signed run timed out", target.triple);
    let parsed = harness_core::parse_output(&outcome.stdout);
    assert!(
        parsed.completed && parsed.failures == 0,
        "{} signed load must complete successfully, stdout bytes: {:02x?}",
        target.triple,
        outcome.stdout,
    );
}

fn assert_signed_tamper_rejected(target: common::DynamicTarget) {
    if !common::require_tool_groups(target.tools) {
        return;
    }
    build_tools();

    let dir = common::temp_dir(&format!("dynamic_signed_tamper_{}", target.triple));
    let image = build_signed_dynamic_image(target, &dir, Some("has-isr"));
    let outcome = common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
    assert!(
        !outcome.timed_out,
        "{} signed tamper must trap, not hang",
        target.triple
    );

    let trap_codes: Vec<u16> = harness_core::parse_records(&outcome.stdout)
        .filter_map(|record| match record {
            harness_core::Record::Diag(payload) => {
                diag_core::DiagRecord::parse(payload).map(|diag| diag.trap_code)
            }
            _ => None,
        })
        .collect();
    assert!(
        trap_codes.contains(&5202),
        "{} signed tamper expected E_SIG_INVALID/5202, got {:?}\nstdout bytes: {:02x?}",
        target.triple,
        trap_codes,
        outcome.stdout,
    );
}

#[test]
fn signed_dynamic_lmod_runs_under_qemu() {
    for target in SIGNED_TARGETS {
        assert_signed_runs(*target);
    }
}

#[test]
fn signed_dynamic_tamper_traps_5202() {
    for target in SIGNED_TARGETS {
        assert_signed_tamper_rejected(*target);
    }
}
