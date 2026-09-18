//! Platform pack discovery, introspection, scaffolding, linting, and the
//! descriptor v2 model (parsing, validation, canonical hash, report).

pub mod desc;
mod config;
mod linker_script;
mod lint;

pub(crate) use config::workspace_root;
pub use config::{
    capabilities_for_selection, capabilities_for_target, discover_platforms, discover_platforms_in,
    info_report, is_qemu_capable_selection, list_report, load_platform_pack, print_info,
    print_list, resolve_platform_selection, run, DebugAgentSection, DebugSection, DeploySection,
    DeployStep, FeatureUnit, IsaEntry, MemoryRegion, MemorySection, MetalSection, PlatformManifest,
    PlatformPack, PlatformSection, ResolvedPlatformSelection, SecureBootSection, TestRung,
    TestSection,
};
pub use linker_script::scaffold_platform_pack;
pub use lint::{
    ensure_build_platform_interface, format_lint_outcome, lint_pack, LintError, LintOutcome,
};
pub use desc::{
    load_descriptor, AccessKind, AllocatorSpec, BarrierKind, Descriptor, DescriptorError,
    DeviceMap, MemoryModel, MemoryRegionSpec, MmioWindow, ReadKind, RegisterRow, ScopedSpec,
    WindowKind, WriteKind, DESCRIPTOR_SCHEMA, E_DESC_INVALID, E_DESC_UNKNOWN_KIND, MMIO_SEM_VER,
    full_mask,
};
pub use desc::compile::{
    ensure_compiled_descriptor, descriptor_file_path, COMPILED_DESC_FILE,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn manifest_from_str(toml: &str) -> PlatformManifest {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn parses_basic_manifest() {
        let manifest = manifest_from_str(
            r#"
[platform]
name = "demo"
compiler-interface = 1

[[platform.isa]]
triple = "armv7m-unknown-none"
arch = "arm"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        );
        assert_eq!(manifest.platform.name, "demo");
        assert_eq!(manifest.platform.compiler_interface, 1);
        assert_eq!(manifest.platform.isa.len(), 1);
        assert_eq!(manifest.platform.isa[0].triple, "armv7m-unknown-none");
        assert_eq!(manifest.metal.startup, "runtime.asm");
        assert_eq!(manifest.test.rung, TestRung::Untested);
    }

    #[test]
    fn renders_summaries() {
        let root = std::env::temp_dir().join("tyu_platform_pack_summary");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("runtime/demo/tests")).unwrap();
        fs::write(root.join("runtime/demo/tests/demo.rs"), "evidence").unwrap();

        let manifest = manifest_from_str(
            r#"
[platform]
name = "demo"
compiler-interface = 1
description = "Demo pack"

[[platform.isa]]
triple = "armv7m-unknown-none"
arch = "arm"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[features.concurrency]
unit = "concurrency.asm"

[capabilities.uart]
glue = "glue/uart"

[deploy]
method = "qemu"
boot = "raw_vectors"

[[deploy.step]]
run = "picotool"
args = ["load"]

[debug]
diag_transport = "semihosting"
rsp = "probe"
probe = "openocd"
probe_config = "demo.cfg"

[test]
rung = "qemu"
target = "crates/tyu/tests/run_qemu_x86.rs"
evidence = "tests/demo.rs"

[test.debug_agent]
supported = true
target = "crates/tyu/tests/escalate.rs"
evidence = "tests/demo.rs"
"#,
        );
        let pack = PlatformPack {
            manifest_path: root.join("runtime/demo/platform.toml"),
            root: root.join("runtime"),
            manifest,
        };
        assert!(pack.isa_summary().contains("armv7m-unknown-none"));
        assert_eq!(pack.capabilities_summary(), "uart");
        assert!(pack.deploy_summary().contains("method=qemu"));
        assert!(pack.debug_summary().contains("diag_transport=semihosting"));
        assert!(pack.test_summary().contains("proven-rung=qemu (automated)"));
        assert!(pack
            .test_summary()
            .contains("target=crates/tyu/tests/run_qemu_x86.rs"));
        assert!(pack.test_summary().contains("evidence=tests/demo.rs"));
        assert!(pack.debug_agent_summary().contains("supported=true"));
        assert!(pack
            .debug_agent_summary()
            .contains("target=crates/tyu/tests/escalate.rs"));
        assert!(pack
            .debug_agent_summary()
            .contains("evidence=tests/demo.rs"));
    }

    #[test]
    fn scaffolds_platform_pack_and_refuses_overwrite() {
        let root = std::env::temp_dir().join("tyu_platform_scaffold");
        let _ = fs::remove_dir_all(&root);

        let pack_root = scaffold_platform_pack(&root, "demo").unwrap();
        assert_eq!(pack_root, root.join("platforms/demo"));
        assert!(pack_root.join("platform.toml").is_file());
        assert!(pack_root.join("metal/runtime.asm").is_file());
        assert!(pack_root.join("metal/link.ld").is_file());
        assert!(pack_root.join("glue/gpio.def").is_file());
        assert!(pack_root.join("glue/gpio.mod").is_file());
        assert!(pack_root.join("glue/uart.def").is_file());
        assert!(pack_root.join("glue/time.def").is_file());
        assert!(pack_root.join("datasheet.md").is_file());
        assert!(pack_root.join("tier.md").is_file());

        let tier = fs::read_to_string(pack_root.join("tier.md")).unwrap();
        assert!(tier.contains("TestRung = untested"));
        let manifest = fs::read_to_string(pack_root.join("platform.toml")).unwrap();
        assert!(manifest.contains("supported = false"));

        let lint = lint_pack(&root, "demo", false).unwrap();
        assert!(lint.errors.is_empty(), "{}", format_lint_outcome(&lint));

        let err = scaffold_platform_pack(&root, "demo").unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn discovers_platform_files() {
        let root = std::env::temp_dir().join("tyu_platform_discovery");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("runtime/foo")).unwrap();
        fs::create_dir_all(root.join("platforms/bar")).unwrap();
        fs::write(
            root.join("runtime/foo/platform.toml"),
            r#"
[platform]
name = "foo"
compiler-interface = 1

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        )
        .unwrap();
        fs::write(
            root.join("runtime/legacy.platform.toml"),
            r#"
[platform]
name = "legacy"
compiler-interface = 1

[[platform.isa]]
triple = "armv7m-unknown-none"
arch = "arm"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        )
        .unwrap();
        fs::write(
            root.join("platforms/bar/platform.toml"),
            r#"
[platform]
name = "bar"
compiler-interface = 1

[[platform.isa]]
triple = "riscv32-unknown-none"
arch = "riscv"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        )
        .unwrap();

        let packs = discover_platforms_in(&root).unwrap();
        let names: Vec<_> = packs.iter().map(|pack| pack.name().to_string()).collect();
        assert_eq!(names, vec!["bar", "foo", "legacy"]);
    }
}
