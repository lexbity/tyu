//! Tests for the project manifest (tyu.toml) parser.

use tyu::project::{parse_project_manifest, resolve_target, ProjectManifest};
use tyu::toml_parser::parse_toml;
use std::path::Path;

#[test]
fn project_section() {
    let toml = r#"
[project]
main = "src/main.mod"
modules = ["src/", "sysroot/"]
"#;
    let entries = parse_toml(toml).unwrap();
    let mut pm = ProjectManifest::default();
    for entry in &entries {
        if entry.section == vec!["project"] {
            match entry.key.as_str() {
                "main" => pm.project.main = Some(
                    match &entry.value { tyu::toml_parser::TomlValue::Str(s) => s.clone(), _ => unreachable!() }
                ),
                "modules" => pm.project.modules = vec!["src/".into(), "sysroot/".into()],
                _ => {}
            }
        }
    }
    assert_eq!(pm.project.main.unwrap(), "src/main.mod");
    assert_eq!(pm.project.modules, vec!["src/", "sysroot/"]);
}

#[test]
fn target_alias() {
    let toml = r#"
[targets.dev]
triple = "linux-x86_64-hosted"
runner = "qemu"
"#;
    let dir = std::env::temp_dir().join("tyu_test_manifest_target");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tyu.toml");
    std::fs::write(&path, toml).unwrap();
    let pm = parse_project_manifest(&path).unwrap();
    let alias = pm.targets.get("dev").unwrap();
    assert_eq!(alias.triple, "linux-x86_64-hosted");
    assert_eq!(alias.runner.as_deref(), Some("qemu"));
}

#[test]
fn toolchain_config() {
    let toml = r#"
[toolchain.armv7m-unknown-none]
as = "arm-none-eabi-as"
ld = "arm-none-eabi-ld"
qemu = "qemu-system-arm"
"#;
    let dir = std::env::temp_dir().join("tyu_test_toolchain");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tyu.toml");
    std::fs::write(&path, toml).unwrap();
    let pm = parse_project_manifest(&path).unwrap();
    let tc = pm.toolchain.get("armv7m-unknown-none").unwrap();
    assert_eq!(tc.asm.as_deref(), Some("arm-none-eabi-as"));
    assert_eq!(tc.ld.as_deref(), Some("arm-none-eabi-ld"));
    assert_eq!(tc.qemu.as_deref(), Some("qemu-system-arm"));
}

#[test]
fn deploy_config() {
    let toml = r#"
[deploy.board]
format = "lmod"
sign = true
encrypt = "device"
key_sign = "env:TYU_SIGN_KEY"
key_encrypt = "env:TYU_ENC_KEK"
"#;
    let dir = std::env::temp_dir().join("tyu_test_deploy");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tyu.toml");
    std::fs::write(&path, toml).unwrap();
    let pm = parse_project_manifest(&path).unwrap();
    let dc = pm.deploy.get("board").unwrap();
    assert_eq!(dc.format.as_deref(), Some("lmod"));
    assert_eq!(dc.sign, Some(true));
    assert_eq!(dc.encrypt.as_deref(), Some("device"));
    assert_eq!(dc.key_sign.as_deref(), Some("env:TYU_SIGN_KEY"));
}

#[test]
fn resolve_alias_to_target() {
    let toml = r#"
[targets.board]
triple = "armv7m-unknown-none"
"#;
    let dir = std::env::temp_dir().join("tyu_test_resolve");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tyu.toml");
    std::fs::write(&path, toml).unwrap();
    let pm = parse_project_manifest(&path).unwrap();
    let target = resolve_target("board", &pm).unwrap();
    assert_eq!(target, codegen_core::Target::ArmV7MUnknownNone);
}

#[test]
fn resolve_direct_triple() {
    let pm = ProjectManifest::default();
    let target = resolve_target("x86_64-unknown-none", &pm).unwrap();
    assert_eq!(target, codegen_core::Target::X86_64UnknownNone);
}

#[test]
fn find_manifest_walks_up() {
    let dir = std::env::temp_dir().join("tyu_test_find_manifest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir.join("sub")).unwrap();
    let path = dir.join("tyu.toml");
    std::fs::write(&path, "[project]\nmain = \"main.mod\"\n").unwrap();

    let found = tyu::project::find_manifest(&dir.join("sub"));
    assert!(found.is_some(), "find_manifest should find tyu.toml in parent");
    assert_eq!(found.unwrap(), path);
}

#[test]
fn find_manifest_returns_none_when_not_found() {
    let dir = std::env::temp_dir().join("tyu_test_no_manifest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let found = tyu::project::find_manifest(&dir);
    assert!(found.is_none());
}
