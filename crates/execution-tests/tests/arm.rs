mod common;

use codegen_core::Target;

#[test]
fn arithmetic_and_stack_pass() {
    if !common::require_tools(&["qemu-system-arm", "arm-none-eabi-as", "arm-none-eabi-ld"]) {
        return;
    }

    let image = common::build_test_image(
        Target::ArmV7MUnknownNone,
        &["arithmetic", "stack_ops", "deep_stack"],
    );
    let result = common::qemu_run(Target::ArmV7MUnknownNone, &image);

    let spec = Target::ArmV7MUnknownNone.spec().qemu.unwrap();
    let summary = common::parse_output(&result.stdout);

    assert!(
        summary.completed,
        "ARM test image did not complete (crashed or hung)"
    );
    assert_eq!(
        summary.failures, 0,
        "ARM test reported {} failure(s)",
        summary.failures
    );
    assert_eq!(
        result.exit_code,
        spec.exit_convention.host_pass_exit(),
        "QEMU ARM exit code {} does not match expected pass code {}",
        result.exit_code,
        spec.exit_convention.host_pass_exit(),
    );

    // High-water assertion: measured peak ≤ re-derived conservative bound
    let slot_bytes = Target::ArmV7MUnknownNone.spec().slot_bytes;
    common::assert_high_water(summary.high_slots, &image, slot_bytes);
}

/// Verify that the ARM metal runtime exports every symbol required by the
/// Runtime ABI contract (abi-contract.md §4.4.1).
#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tools(&["arm-none-eabi-as", "arm-none-eabi-nm"]) {
        return;
    }

    let target = Target::ArmV7MUnknownNone;
    let out_dir = common::temp_dir("arm_runtime_symcheck");
    let runtime_o = common::assemble_runtime(target, &out_dir);

    let output = std::process::Command::new("arm-none-eabi-nm")
        .arg("--defined-only")
        .arg("-o")
        .arg(&runtime_o)
        .output()
        .expect("arm-none-eabi-nm invocation failed");
    assert!(output.status.success(), "arm-none-eabi-nm failed");
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
            "ARM runtime.asm is missing required symbol `{}` (abi-contract §4.4.1)",
            sym,
        );
    }
}
