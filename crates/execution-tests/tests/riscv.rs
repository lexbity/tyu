//! RISC-V RV32 execution tests via `tyu test` driver.

mod common;

use std::process::Command;

fn build_langc() {
    let _ = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status();
}

#[test]
fn arithmetic_and_stack_pass() {
    if !common::require_tools(&["langc", "riscv64-unknown-elf-as", "riscv64-unknown-elf-ld", "qemu-system-riscv32"]) {
        return;
    }
    build_langc();

    let output = Command::new(common::tyu_exe())
        .args([
            "test",
            "--target=riscv32-unknown-none",
            &format!("--manifest={}", common::fixtures_manifest().display()),
        ])
        .output()
        .expect("tyu test");

    assert!(
        output.status.success(),
        "tyu test riscv32-unknown-none failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tools(&["riscv64-unknown-elf-as", "riscv64-unknown-elf-nm"]) {
        return;
    }

    let target = codegen_core::Target::RiscV32UnknownNone;
    let out_dir = common::temp_dir("riscv_runtime_symcheck");
    let runtime_o = common::assemble_runtime(target, &out_dir);

    let output = Command::new("riscv64-unknown-elf-nm")
        .arg("--defined-only")
        .arg("-o")
        .arg(&runtime_o)
        .output()
        .expect("riscv64-unknown-elf-nm invocation failed");
    assert!(output.status.success(), "riscv64-unknown-elf-nm failed");
    let stdout = String::from_utf8_lossy(&output.stdout);

    let required = &[
        "__lang_start",
        "__lang_trap",
        "__lang_trap_loc",
        "__stack_overflow",
        "__lang_ds_base",
        "__lang_ds_limit",
        "__lang_ds_high",
        "__lang_expected_abi_hash",
        "__lang_modpack_start",
        "__lang_modpack_end",
    ];

    for sym in required {
        assert!(
            stdout.contains(*sym),
            "RISC-V runtime.asm is missing required symbol `{}` (abi-contract §4.4.1)",
            sym,
        );
    }
}
