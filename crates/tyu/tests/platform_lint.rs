//! Tests for `tyu platform lint`.

use std::fs;
use std::path::Path;

use tyu::platform::{discover_platforms_in, format_lint_outcome, lint_pack};

const X86_ABI_HASH: u64 = 0x50FB_AC4F_4E87_016C; // compute_abi_hash(x86_64, MODINFO_VER=4)

fn x86_abi_hash_literal() -> String {
    format!("0x{:016x}", X86_ABI_HASH)
}

fn write_file(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn write_pack(root: &Path, manifest: &str, startup: &str, linker: Option<&str>) {
    write_file(&root.join("platforms/demo/platform.toml"), manifest);
    write_file(&root.join("platforms/demo/runtime.asm"), startup);
    write_file(&root.join("runtime/runtime.asm"), startup);
    write_file(
        &root.join("crates/tyu/tests/run_qemu_x86.rs"),
        "#[test] fn run_qemu_pass() {}",
    );
    if let Some(linker) = linker {
        write_file(&root.join("platforms/demo/link.ld"), linker);
        write_file(&root.join("runtime/link.ld"), linker);
    }
    write_file(&root.join("platforms/demo/tests/demo.rs"), "demo evidence");
    write_file(
        &root.join("platforms/demo/concurrency.asm"),
        "; feature unit",
    );
    write_file(&root.join("runtime/tests/demo.rs"), "demo evidence");
    write_file(&root.join("runtime/concurrency.asm"), "; feature unit");
}

fn base_manifest() -> String {
    r#"
[platform]
name = "demo"
compiler-interface = 1
description = "demo pack"

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true
expected_abi_hash = "__ABI_HASH__"

[metal]
path = "."
startup = "runtime.asm"
linker = ""

[features.concurrency]
unit = "concurrency.asm"

[deploy]
method = "elf-qemu"
boot = "raw_vectors"

[test]
rung = "qemu"
target = "crates/tyu/tests/run_qemu_x86.rs"
evidence = "tests/demo.rs"
"#
    .replace("__ABI_HASH__", &x86_abi_hash_literal())
}

fn base_manifest_with_debug_agent() -> String {
    format!(
        r#"{base}

[test.debug_agent]
supported = true
target = "crates/tyu/tests/escalate.rs"
evidence = "tests/escalate.md"
"#,
        base = base_manifest()
    )
}

fn base_startup() -> &'static str {
    "\
public __lang_start\n\
public __lang_trap\n\
public __lang_ds_base\n\
public __lang_ds_limit\n\
public __lang_ds_high\n\
public __lang_expected_abi_hash\n"
}

#[test]
fn lint_valid_pack_passes() {
    let root = std::env::temp_dir().join("tyu_platform_lint_valid");
    let _ = fs::remove_dir_all(&root);
    write_pack(&root, &base_manifest(), base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert!(
        outcome.errors.is_empty(),
        "{}",
        format_lint_outcome(&outcome)
    );
}

#[test]
fn lint_reports_interface_mismatch() {
    let root = std::env::temp_dir().join("tyu_platform_lint_iface");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest().replace("compiler-interface = 1", "compiler-interface = 999");
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5401);
}

#[test]
fn lint_reports_missing_symbol() {
    let root = std::env::temp_dir().join("tyu_platform_lint_symbol");
    let _ = fs::remove_dir_all(&root);
    let startup = "\
public __lang_start\n\
public __lang_trap\n\
public __lang_ds_base\n\
public __lang_ds_limit\n\
public __lang_ds_high\n";
    write_pack(&root, &base_manifest(), startup, None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5402);
}

#[test]
fn lint_reports_missing_memory_region() {
    let root = std::env::temp_dir().join("tyu_platform_lint_memory");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest()
        .replace("[metal]\npath = \".\"\nstartup = \"runtime.asm\"\nlinker = \"\"\n", "[metal]\npath = \".\"\nstartup = \"runtime.asm\"\nlinker = \"link.ld\"\n\n[memory]\nflash = { name = \"FLASH\", origin = 0x0, length = 0x1000 }\nsram = { name = \"SRAM\", origin = 0x1000, length = 0x1000 }\nds_region = \"SRAM\"\nds_size = 0x4000\n");
    write_pack(
        &root,
        &manifest,
        base_startup(),
        Some(
            "\
ENTRY(__lang_start)\n\
MEMORY\n\
{\n\
    FLASH (rx) : ORIGIN = 0x0, LENGTH = 0x1000\n\
}\n\
SECTIONS { .text : { *(.text) } > FLASH }\n",
        ),
    );

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5403);
}

#[test]
fn lint_reports_missing_feature_unit() {
    let root = std::env::temp_dir().join("tyu_platform_lint_feature");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest().replace("unit = \"concurrency.asm\"", "unit = \"missing.asm\"");
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5404);
}

#[test]
fn lint_reports_capability_glue_missing() {
    let root = std::env::temp_dir().join("tyu_platform_lint_capability");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest() + "\n[capabilities.gpio]\nglue = \"glue/gpio\"\n";
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5405);
}

#[test]
fn lint_reports_abi_hash_mismatch() {
    let root = std::env::temp_dir().join("tyu_platform_lint_hash");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest().replace(
        &format!("expected_abi_hash = \"{}\"", x86_abi_hash_literal()),
        "expected_abi_hash = \"0x0\"",
    );
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5406);
}

#[test]
fn lint_reports_unbacked_testrung() {
    let root = std::env::temp_dir().join("tyu_platform_lint_rung");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest().replace("evidence = \"tests/demo.rs\"\n", "");
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5407);
}

#[test]
fn lint_reports_unbacked_debug_agent() {
    let root = std::env::temp_dir().join("tyu_platform_lint_debug_agent");
    let _ = fs::remove_dir_all(&root);
    let manifest =
        base_manifest_with_debug_agent().replace("evidence = \"tests/escalate.md\"\n", "");
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5412);
}

#[test]
fn lint_reports_debug_agent_evidence_missing_on_disk() {
    let root = std::env::temp_dir().join("tyu_platform_lint_debug_agent_missing");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest_with_debug_agent();
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5412);
    assert!(
        outcome.errors[0].detail.contains("debug-agent evidence"),
        "{}",
        format_lint_outcome(&outcome)
    );
}

#[test]
fn lint_reports_deploy_recipe_invalid() {
    let root = std::env::temp_dir().join("tyu_platform_lint_deploy");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest().replace("method = \"elf-qemu\"", "method = \"bogus\"");
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5408);
}

#[test]
fn lint_all_collects_multiple_failures() {
    let root = std::env::temp_dir().join("tyu_platform_lint_all");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest()
        .replace("compiler-interface = 1", "compiler-interface = 999")
        .replace(
            &format!("expected_abi_hash = \"{}\"", x86_abi_hash_literal()),
            "expected_abi_hash = \"0x0\"",
        )
        .replace("method = \"elf-qemu\"", "method = \"bogus\"")
        .replace("evidence = \"tests/demo.rs\"\n", "");
    let startup = "\
public __lang_start\n\
public __lang_trap\n\
public __lang_ds_base\n\
public __lang_ds_limit\n\
";
    write_pack(&root, &manifest, startup, None);

    let outcome = lint_pack(&root, "demo", true).unwrap();
    let codes: Vec<_> = outcome.errors.iter().map(|e| e.code).collect();
    assert!(codes.contains(&5401));
    assert!(codes.contains(&5402));
    assert!(codes.contains(&5406));
    assert!(codes.contains(&5407));
    assert!(codes.contains(&5408));
}

#[test]
fn consolidated_pack_prefers_platform_layout() {
    let root = std::env::temp_dir().join("tyu_platform_lint_duplicate");
    let _ = fs::remove_dir_all(&root);
    write_pack(&root, &base_manifest(), base_startup(), None);

    let mut manifest = base_manifest();
    manifest = manifest.replace(
        "description = \"demo pack\"",
        "description = \"demo consolidated pack\"",
    );
    write_file(&root.join("platforms/demo/platform.toml"), &manifest);
    write_file(&root.join("platforms/demo/runtime.asm"), base_startup());
    write_file(&root.join("platforms/demo/link.ld"), "ENTRY(__lang_start)\nMEMORY { FLASH (rx) : ORIGIN = 0x0, LENGTH = 0x1000 }\nSECTIONS { }\n");

    let packs = discover_platforms_in(&root).unwrap();
    assert_eq!(packs.len(), 1);
    assert!(packs[0].display_path().starts_with("platforms/demo"));
    assert!(packs[0]
        .manifest
        .platform
        .description
        .as_deref()
        .unwrap()
        .contains("consolidated"));
}

#[test]
fn lint_reports_unbacked_test_target() {
    let root = std::env::temp_dir().join("tyu_platform_lint_target");
    let _ = fs::remove_dir_all(&root);
    let manifest = base_manifest().replace("target = \"crates/tyu/tests/run_qemu_x86.rs\"\n", "");
    write_pack(&root, &manifest, base_startup(), None);

    let outcome = lint_pack(&root, "demo", false).unwrap();
    assert_eq!(outcome.errors[0].code, 5407);
}
