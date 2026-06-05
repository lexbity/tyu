//! x86_64 bare-metal execution tests via `tyu test` driver.

mod common;

use std::process::Command;

fn build_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status().expect("cargo build");
    assert!(s.success(), "cargo build failed");
}

#[test]
fn arithmetic_and_stack_pass() {
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_langc();

    let output = Command::new(common::tyu_exe())
        .args([
            "test",
            "--target=x86_64-unknown-none",
            &format!("--manifest={}", common::fixtures_manifest().display()),
        ])
        .output()
        .expect("tyu test");

    assert!(
        output.status.success(),
        "tyu test x86_64-unknown-none failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Verify that the x86_64 metal runtime exports every symbol required by the
/// Runtime ABI contract (abi-contract.md §4.4.1).
#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tools(&["fasm", "nm"]) {
        return;
    }

    let target = codegen_core::Target::X86_64UnknownNone;
    let out_dir = common::temp_dir("runtime_symcheck");
    let runtime_o = common::assemble_runtime(target, &out_dir);

    let output = Command::new("nm")
        .arg("--defined-only")
        .arg("-o")
        .arg(&runtime_o)
        .output()
        .expect("nm invocation failed");
    assert!(output.status.success(), "nm failed");
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
            "runtime.asm is missing required symbol `{}` (abi-contract §4.4.1)",
            sym,
        );
    }
}
