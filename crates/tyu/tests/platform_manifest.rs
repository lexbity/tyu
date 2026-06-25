//! Tests for `tyu platform` discovery and reporting.

use std::fs;
use std::path::Path;

use tyu::platform::{discover_platforms_in, info_report, list_report};

fn write_manifest(
    path: &Path,
    name: &str,
    triple: &str,
    arch: &str,
    rung: &str,
    target: &str,
    evidence: &str,
) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            r#"
[platform]
name = "{name}"
compiler-interface = 1
description = "{name} pack"

[[platform.isa]]
triple = "{triple}"
arch = "{arch}"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[features.concurrency]
unit = "concurrency.asm"

[deploy]
method = "elf-qemu"
boot = "raw_vectors"

[debug]
diag_transport = "semihosting"
rsp = "probe"
probe = "openocd"
probe_config = "{name}.cfg"

[test]
rung = "{rung}"
target = "{target}"
evidence = "{evidence}"
"#
        ),
    )
    .unwrap();
    if !target.is_empty() {
        let target_path = path.parent().unwrap().join(target);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(target_path, "target").unwrap();
    }
    if !evidence.is_empty() {
        let evidence_path = path.parent().unwrap().join(evidence);
        if let Some(parent) = evidence_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(evidence_path, "evidence").unwrap();
    }
}

fn write_manifest_with_debug_agent(
    path: &Path,
    name: &str,
    triple: &str,
    arch: &str,
    rung: &str,
    target: &str,
    evidence: &str,
    debug_target: &str,
    debug_evidence: &str,
) {
    write_manifest(path, name, triple, arch, rung, target, evidence);
    if !debug_evidence.is_empty() {
        let debug_evidence_path = path.parent().unwrap().join(debug_evidence);
        if let Some(parent) = debug_evidence_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(debug_evidence_path, "debug evidence").unwrap();
    }
    fs::write(
        path,
        format!(
            r#"
[platform]
name = "{name}"
compiler-interface = 1
description = "{name} pack"

[[platform.isa]]
triple = "{triple}"
arch = "{arch}"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[features.concurrency]
unit = "concurrency.asm"

[deploy]
method = "elf-qemu"
boot = "raw_vectors"

[debug]
diag_transport = "semihosting"
rsp = "probe"
probe = "openocd"
probe_config = "{name}.cfg"

[test]
rung = "{rung}"
target = "{target}"
evidence = "{evidence}"

[test.debug_agent]
supported = true
target = "{debug_target}"
evidence = "{debug_evidence}"
"#
        ),
    )
    .unwrap();
}

fn write_consolidated_manifest(path: &Path, name: &str, triple: &str, arch: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            r#"
[platform]
name = "{name}"
compiler-interface = 1
description = "{name} consolidated pack"

[[platform.isa]]
triple = "{triple}"
arch = "{arch}"
default = true

[metal]
path = "metal"
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#
        ),
    )
    .unwrap();
}

#[test]
fn discover_runtime_and_platform_packs() {
    let root = std::env::temp_dir().join("tyu_platform_manifest_discover");
    let _ = fs::remove_dir_all(&root);

    write_manifest(
        &root.join("runtime/armv7m-unknown-none/platform.toml"),
        "armv7m-unknown-none",
        "armv7m-unknown-none",
        "arm",
        "untested",
        "",
        "tests/arm.md",
    );
    write_manifest(
        &root.join("runtime/riscv32-unknown-none/platform.toml"),
        "riscv32-unknown-none",
        "riscv32-unknown-none",
        "riscv",
        "untested",
        "",
        "tests/riscv.md",
    );
    write_manifest(
        &root.join("runtime/linux-x86_64-hosted.platform.toml"),
        "linux-x86_64-hosted",
        "x86_64-unknown-linux-gnu",
        "x86_64",
        "qemu",
        "crates/tyu/tests/run_qemu_x86.rs",
        "tests/hosted.md",
    );
    write_manifest(
        &root.join("platforms/rp2350/platform.toml"),
        "rp2350",
        "armv7m-unknown-none",
        "arm",
        "hardware",
        "crates/tyu/tests/platform_hil.rs",
        "docs/hil/rp2350.md",
    );

    let packs = discover_platforms_in(&root).unwrap();
    let names: Vec<_> = packs.iter().map(|p| p.name().to_string()).collect();
    assert_eq!(
        names,
        vec![
            "armv7m-unknown-none",
            "linux-x86_64-hosted",
            "riscv32-unknown-none",
            "rp2350",
        ]
    );
}

#[test]
fn list_output_includes_summary_fields() {
    let root = std::env::temp_dir().join("tyu_platform_manifest_list");
    let _ = fs::remove_dir_all(&root);

    write_manifest(
        &root.join("runtime/demo.platform.toml"),
        "demo",
        "x86_64-unknown-none",
        "x86_64",
        "qemu",
        "crates/tyu/tests/run_qemu_x86.rs",
        "tests/demo.md",
    );

    let packs = discover_platforms_in(&root).unwrap();
    assert_eq!(packs.len(), 1);
    assert_eq!(packs[0].name(), "demo");
    let rendered = list_report(&root).unwrap();
    assert!(rendered.contains("demo"));
    assert!(rendered.contains("compiler-interface=1"));
    assert!(rendered.contains("rung=qemu"));
}

#[test]
fn info_output_resolves_pack_and_isa_filter() {
    let root = std::env::temp_dir().join("tyu_platform_manifest_info");
    let _ = fs::remove_dir_all(&root);

    write_manifest(
        &root.join("runtime/demo.platform.toml"),
        "demo",
        "armv7m-unknown-none",
        "arm",
        "hardware",
        "crates/tyu/tests/platform_hil.rs",
        "docs/hil/demo.md",
    );

    let pack = discover_platforms_in(&root).unwrap();
    assert_eq!(pack[0].display_path(), "runtime/demo.platform.toml");
    assert!(pack[0].deploy_summary().contains("method=elf-qemu"));

    let rendered = info_report(&root, "demo", Some("arm")).unwrap();
    assert!(rendered.contains("platform demo"));
    assert!(rendered.contains("manifest: runtime/demo.platform.toml"));
    assert!(rendered.contains("test: proven-rung=hardware (manual)"));
    assert!(rendered.contains("target=crates/tyu/tests/platform_hil.rs"));
    assert!(rendered.contains("evidence=docs/hil/demo.md"));
}

#[test]
fn info_output_shows_debug_agent_summary() {
    let root = std::env::temp_dir().join("tyu_platform_manifest_debug_agent");
    let _ = fs::remove_dir_all(&root);

    write_manifest_with_debug_agent(
        &root.join("runtime/demo.platform.toml"),
        "demo",
        "x86_64-unknown-none",
        "x86_64",
        "qemu",
        "crates/tyu/tests/run_qemu_x86.rs",
        "tests/demo.md",
        "crates/tyu/tests/escalate.rs",
        "tests/escalate.md",
    );

    let rendered = info_report(&root, "demo", None).unwrap();
    assert!(rendered.contains("debug-agent: supported=true"));
    assert!(rendered.contains("target=crates/tyu/tests/escalate.rs"));
    assert!(rendered.contains("evidence=tests/escalate.md"));
}

#[test]
fn consolidated_platform_pack_wins_over_runtime_layout() {
    let root = std::env::temp_dir().join("tyu_platform_manifest_consolidated");
    let _ = fs::remove_dir_all(&root);

    write_manifest(
        &root.join("runtime/demo.platform.toml"),
        "demo",
        "x86_64-unknown-none",
        "x86_64",
        "untested",
        "",
        "",
    );
    write_consolidated_manifest(
        &root.join("platforms/demo/platform.toml"),
        "demo",
        "x86_64-unknown-none",
        "x86_64",
    );

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
