mod common;

use codegen_core::Target;

fn check_tools() -> bool {
    common::tool_available("qemu-system-x86_64")
        && common::tool_available("fasm")
        && common::tool_available("ld")
}

#[test]
fn arithmetic_and_stack_pass() {
    if !check_tools() {
        eprintln!("SKIP: required tools not available (qemu-system-x86_64, fasm, ld)");
        return;
    }

    let image = common::build_test_image(
        Target::X86_64UnknownNone,
        &["arithmetic", "stack_ops"],
    );
    let result = common::qemu_run(Target::X86_64UnknownNone, &image);

    let spec = Target::X86_64UnknownNone.spec().qemu.unwrap();
    let (failures, completed) = common::parse_output(&result.stdout);

    assert!(completed, "test image did not complete (crashed or hung)");
    assert_eq!(failures, 0, "test reported {failures} failure(s)");
    assert_eq!(
        result.exit_code,
        spec.exit_convention.host_pass_exit(),
        "QEMU exit code {} does not match expected pass code {}",
        result.exit_code,
        spec.exit_convention.host_pass_exit(),
    );
}
