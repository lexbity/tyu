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
    build_mutated_dynamic_image_with_mod(target, mutation, dir, PASS_MOD)
}

fn build_mutated_dynamic_image_with_mod(
    target: common::DynamicTarget,
    mutation: &str,
    dir: &Path,
    module: &str,
) -> PathBuf {
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, module).unwrap();
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
        ])
        .args(
            // The MMIO module binds `board.scratch`, which needs the platform
            // descriptor (E3640 without it). The non-MMIO PASS_MOD also
            // compiles fine with a descriptor present.
            ["--platform", target.triple],
        )
        .arg(&main_mod)
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

/// A module that touches the arm/riscv `board.scratch` window, so packing
/// produces a `MmioWindowBase` reloc site the loader binds and re-validates.
const MMIO_PASS_MOD: &str = "module Main;\n\
import platform/testio { testio.write-byte };\n\
register-map Scratch\n\
  0x00 A u32 rw\n\
end;\n\
const scratch = Scratch @ board.scratch;\n\
: main ( -- i64 )\n\
  &!scratch.A 1 as u32 !u32\n\
  &scratch.A @u32 as i64 1 == [ 83 testio.write-byte ] [ 70 testio.write-byte ] if\n\
  10 testio.write-byte\n\
  0 ;\n\
export { main };\nend;\n";

/// P6 `check_window_base`: a module whose window-base reloc site was bound to
/// a different base than the device's descriptor-derived base must be
/// rejected (5221) — no silent binding to a foreign window geometry.
#[test]
fn dynamic_window_base_mismatch_traps_5221() {
    for target in common::DYNAMIC_TARGETS {
        if target.target == codegen_core::Target::X86_64UnknownNone {
            continue; // x86's emulated window has no binding-time reloc
        }
        if !common::require_tool_groups(target.tools) {
            return;
        }
        build_tools();
        let dir =
            common::temp_dir(&format!("dynamic_negative_{}_window-base-mismatch", target.triple));
        let image =
            build_mutated_dynamic_image_with_mod(*target, "window-base-mismatch", &dir, MMIO_PASS_MOD);
        let outcome =
            common::run_with_product_runner(target.target, &image, Duration::from_secs(10));
        assert!(
            !outcome.timed_out,
            "{} window-base-mismatch must trap, not hang",
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
            trap_codes.contains(&5221),
            "{} window-base-mismatch expected loader trap 5221, got {:?}\nstdout bytes: {:02x?}",
            target.triple,
            trap_codes,
            outcome.stdout,
        );
    }
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
