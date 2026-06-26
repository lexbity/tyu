//! Dynamic-loader negative corpus under QEMU.
//!
//! These tests build a normal dynamic firmware, mutate the embedded `.lmod`
//! after packing and before `.modpack` embedding, then boot the firmware under
//! QEMU and assert the loader's exact 52xx trap code.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const PASS_MOD: &str = "module Main;\n\
import platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\n\
export { main };\nend;\n";

fn build_tools() {
    let status = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "langc", "-p", "tyu"])
        .status()
        .expect("cargo build langc tyu");
    assert!(status.success(), "cargo build langc tyu failed");
}

fn build_mutated_dynamic_image(
    target: common::DynamicTarget,
    mutation: &str,
    dir: &Path,
) -> PathBuf {
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, PASS_MOD).unwrap();
    let out_dir = dir.join(format!("{}_{}", target.triple, mutation).replace('-', "_"));
    let sysroot = common::workspace_root().join("sysroot");

    let output = Command::new(common::tyu_exe())
        .current_dir(common::workspace_root())
        .env("TYU_TEST_MUTATE_LMOD", mutation)
        .args([
            "build",
            "--mode=dynamic",
            &format!("--target={}", target.triple),
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu build --mode=dynamic");
    assert!(
        output.status.success(),
        "{} dynamic mutated build failed for {mutation}:\nstdout:\n{}\nstderr:\n{}",
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

fn assert_loader_trap(target: common::DynamicTarget, mutation: &str, expected_code: u16) {
    if !common::require_tool_groups(target.tools) {
        return;
    }
    build_tools();

    let dir = common::temp_dir(&format!("dynamic_negative_{}_{}", target.triple, mutation));
    let image = build_mutated_dynamic_image(target, mutation, &dir);
    let outcome = common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
    assert!(
        !outcome.timed_out,
        "{} {mutation} must trap, not hang",
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
        trap_codes.contains(&expected_code),
        "{} {mutation} expected loader trap {expected_code}, got {:?}\nstdout bytes: {:02x?}",
        target.triple,
        trap_codes,
        outcome.stdout,
    );
}

#[test]
fn dynamic_bad_container_traps_5201() {
    for target in common::DYNAMIC_TARGETS {
        assert_loader_trap(*target, "truncate", 5201);
    }
}

#[test]
fn dynamic_abi_mismatch_traps_5200() {
    for target in common::DYNAMIC_TARGETS {
        assert_loader_trap(*target, "abi-zero", 5200);
    }
}

#[test]
fn dynamic_isr_declaration_traps_5203() {
    for target in common::DYNAMIC_TARGETS {
        assert_loader_trap(*target, "has-isr", 5203);
    }
}

#[test]
fn dynamic_unsupported_reloc_traps_5204() {
    for target in common::DYNAMIC_TARGETS {
        assert_loader_trap(*target, "reloc-unsupported", 5204);
    }
}

#[test]
fn dynamic_unresolved_symbol_traps_5205() {
    for target in common::DYNAMIC_TARGETS {
        assert_loader_trap(*target, "symbol-unresolved", 5205);
    }
}
