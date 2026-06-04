mod common;

use codegen_core::Target;

#[test]
fn arithmetic_and_stack_pass() {
    if !common::require_tools(&["qemu-system-x86_64", "fasm", "ld"]) {
        return;
    }

    let image = common::build_test_image(
        Target::X86_64UnknownNone,
        &["arithmetic", "stack_ops", "deep_stack"],
    );
    let result = common::qemu_run(Target::X86_64UnknownNone, &image);

    let spec = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let summary = common::parse_output(&result.stdout);

    common::assert_qemu_ok(&result, &summary, spec);

    // High-water assertion: measured peak ≤ re-derived conservative bound
    let slot_bytes = Target::X86_64UnknownNone.spec().slot_bytes;
    common::assert_high_water(summary.high_slots, &image, slot_bytes);
}

/// Verify that the x86_64 metal runtime exports every symbol required by the
/// Runtime ABI contract (abi-contract.md §4.4.1).
#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tools(&["fasm", "nm"]) {
        return;
    }

    let target = Target::X86_64UnknownNone;
    let out_dir = common::temp_dir("runtime_symcheck");
    let runtime_o = common::assemble_runtime(target, &out_dir);

    let output = std::process::Command::new("nm")
        .arg("--defined-only")
        .arg("-o")
        .arg(&runtime_o)
        .output()
        .expect("nm invocation failed");
    assert!(output.status.success(), "nm failed");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Required from abi-contract.md §4.4.1 (mandatory column).
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
            "runtime.asm is missing required symbol `{}` (abi-contract §4.4.1)",
            sym,
        );
    }
}
