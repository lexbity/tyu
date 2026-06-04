mod common;

use codegen_core::Target;

#[test]
fn arithmetic_and_stack_pass() {
    if !common::require_tools(&["qemu-system-riscv32", "riscv64-unknown-elf-as", "riscv64-unknown-elf-ld"]) {
        return;
    }

    let image = common::build_test_image(
        Target::RiscV32UnknownNone,
        &["arithmetic", "stack_ops", "deep_stack"],
    );
    let result = common::qemu_run(Target::RiscV32UnknownNone, &image);

    let spec = Target::RiscV32UnknownNone.spec().qemu.unwrap();
    let summary = common::parse_output(&result.stdout);

    assert!(
        summary.completed,
        "RISC-V test image did not complete (crashed or hung)"
    );
    assert_eq!(
        summary.failures, 0,
        "RISC-V test reported {} failure(s)",
        summary.failures
    );
    assert_eq!(
        result.exit_code,
        spec.exit_convention.host_pass_exit(),
        "QEMU exit code {} does not match expected pass code {}",
        result.exit_code,
        spec.exit_convention.host_pass_exit(),
    );

    let slot_bytes = Target::RiscV32UnknownNone.spec().slot_bytes;
    common::assert_high_water(summary.high_slots, &image, slot_bytes);
}

#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tools(&["riscv64-unknown-elf-as", "riscv64-unknown-elf-nm"]) {
        return;
    }

    let target = Target::RiscV32UnknownNone;
    let out_dir = common::temp_dir("riscv_runtime_symcheck");
    let runtime_o = common::assemble_runtime(target, &out_dir);

    let output = std::process::Command::new("riscv64-unknown-elf-nm")
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
