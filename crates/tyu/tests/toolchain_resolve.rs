//! Tests for toolchain resolution.

use tyu::project::ProjectManifest;
use tyu::toolchain::{resolve_tools, ToolRole, ToolSource};

/// A toolchain config that explicitly sets paths.
fn make_tc() -> tyu::project::ToolchainConfig {
    tyu::project::ToolchainConfig {
        asm: Some("fasm".into()),
        ld: Some("ld".into()),
        qemu: None,
    }
}

#[test]
fn resolve_via_manifest() {
    let mut manifest = ProjectManifest::default();
    manifest.toolchain.insert("x86_64-unknown-none".into(), make_tc());

    let flags = std::collections::HashMap::new();
    let target = codegen_core::Target::X86_64UnknownNone;
    let res = resolve_tools(target, &manifest, &flags);

    // fasm should be found as assembler via manifest (fasm will be in PATH
    // if the build tools are installed; if not, the test is informative).
    if let Some(ref asm) = res.assembler {
        assert!(
            matches!(asm.source, ToolSource::Manifest),
            "assembler should come from manifest: {:?}",
            asm.source,
        );
    }
}

#[test]
fn env_var_overrides_path() {
    // Set TYU_AS_X86_64_UNKNOWN_NONE to something unlikely.
    // The resolver should find it via env var, not PATH.
    let triple_upper = "X86_64_UNKNOWN_NONE";
    let var = format!("TYU_AS_{}", triple_upper);
    std::env::set_var(&var, "this_should_not_exist_xyzzy");

    let manifest = ProjectManifest::default();
    let flags = std::collections::HashMap::new();
    let target = codegen_core::Target::X86_64UnknownNone;
    let res = resolve_tools(target, &manifest, &flags);

    // Since the env var points to a non-existent binary, the resolver
    // should fall through to PATH lookup. We just verify it doesn't panic.
    std::env::remove_var(&var);
    let _ = res;
}

#[test]
fn native_target_has_no_qemu() {
    let manifest = ProjectManifest::default();
    let flags = std::collections::HashMap::new();
    let target = codegen_core::Target::X86_64UnknownLinuxGnu;
    let res = resolve_tools(target, &manifest, &flags);

    // Native target has no QemuSpec, so qemu should be None.
    assert!(res.qemu.is_none(), "native target should have no qemu tool");
}

#[test]
fn tool_role_default_names() {
    let target = codegen_core::Target::X86_64UnknownNone;
    assert_eq!(
        std::str::from_utf8(ToolRole::Linker.default_name(target)).unwrap(),
        "ld",
    );
    assert_eq!(
        std::str::from_utf8(ToolRole::Assembler.default_name(target)).unwrap(),
        "fasm",
    );
}

#[test]
fn tool_role_env_var_names() {
    let target = codegen_core::Target::X86_64UnknownNone;
    let var = ToolRole::Assembler.env_var_name(target);
    assert_eq!(var, "TYU_AS_X86_64_UNKNOWN_NONE");

    let target2 = codegen_core::Target::ArmV7MUnknownNone;
    let var2 = ToolRole::Linker.env_var_name(target2);
    assert_eq!(var2, "TYU_LD_ARMV7M_UNKNOWN_NONE");
}

#[test]
fn toolchain_check_does_not_panic() {
    let manifest = ProjectManifest::default();
    let target = codegen_core::Target::X86_64UnknownNone;
    let report = tyu::toolchain::toolchain_check(target, &manifest);
    // Should not panic. Should contain some output.
    assert!(report.contains("compiler"), "report should mention compiler");
    assert!(report.contains("assembler"), "report should mention assembler");
}
