//! Hosted (x86_64-unknown-linux-gnu) codegen tests.

mod common;

use std::path::Path;
use std::process::Command;

/// BUG-004: the x86_64 backend must emit the resource symbol address for
/// `&!Res` inside `lock` instead of returning `UnsupportedAddrOf` (E8008),
/// and must declare the resource globals in the module object.
#[test]
fn resource_lock_addr_of_emitted() {
    if !common::require_tools(&["langc", "fasm"]) {
        return;
    }
    let s = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");

    let dir = common::temp_dir("resource_lock_hosted");
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("resource_lock_hosted.mod");

    common::langc_compile(
        codegen_core::Target::X86_64UnknownLinuxGnu,
        &src_path,
        &dir,
        true, // --lib: runner provides main; we only inspect asm shape
    );

    let asm_path = dir.join("ResourceLockHosted.asm");
    let asm = std::fs::read_to_string(&asm_path).expect("x86_64 assembly must be generated");
    assert!(
        asm.contains("mov rax, r_"),
        "resource AddrOf must load the resource symbol address;\n{asm}"
    );
    assert!(
        asm.contains("section '.bss'")
            && asm.contains("r_")
            && asm.contains("dq 0"),
        "module must declare its resource globals;\n{asm}"
    );
}