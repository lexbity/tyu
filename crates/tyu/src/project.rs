//! Project manifest (`tyu.toml`) parsing and resolution.
//!
//! Provides types for the `[project]`, `[targets.*]`, `[toolchain.*]`, and
//! `[deploy.*]` sections, plus helpers to locate the manifest and resolve
//! target aliases.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::toml_parser::{parse_toml, TomlEntry, TomlValue};

/// The complete project manifest.
#[derive(Clone, Debug, Default)]
pub struct ProjectManifest {
    pub project: ProjectSection,
    pub targets: HashMap<String, TargetAlias>,
    pub toolchain: HashMap<String, ToolchainConfig>,
    pub deploy: HashMap<String, DeployConfig>,
}

/// `[project]` section.
#[derive(Clone, Debug, Default)]
pub struct ProjectSection {
    pub main: Option<String>,
    pub modules: Vec<String>,
}

/// `[targets.<name>]` — a named target alias.
#[derive(Clone, Debug)]
pub struct TargetAlias {
    pub triple: String,
    pub runner: Option<String>,
}

/// `[toolchain.<triple>]` — tool overrides for a target triple.
#[derive(Clone, Debug, Default)]
pub struct ToolchainConfig {
    pub asm: Option<String>,
    pub ld: Option<String>,
    pub qemu: Option<String>,
}

/// `[deploy.<name>]` — deployment configuration.
#[derive(Clone, Debug, Default)]
pub struct DeployConfig {
    pub format: Option<String>,
    pub sign: Option<bool>,
    pub encrypt: Option<String>,
    pub key_sign: Option<String>,
    pub key_encrypt: Option<String>,
}

/// Locate `tyu.toml` starting from `dir` and walking up.
/// Returns the path to the manifest file, or `None` if not found.
pub fn find_manifest(start_dir: &Path) -> Option<PathBuf> {
    let mut current = Some(start_dir.to_path_buf());
    while let Some(dir) = current {
        let candidate = dir.join("tyu.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    None
}

/// Parse a `tyu.toml` file into a `ProjectManifest`.
pub fn parse_project_manifest(path: &Path) -> Result<ProjectManifest, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("reading '{}': {}", path.display(), e))?;
    let entries = parse_toml(&text)?;
    let mut pm = ProjectManifest::default();

    for entry in &entries {
        let sec = &entry.section;
        if sec.is_empty() {
            continue;
        }
        match sec[0].as_str() {
            "project" => {
                process_project(&mut pm.project, entry);
            }
            "targets" if sec.len() >= 2 => {
                let name = sec[1].clone();
                let alias = pm.targets.entry(name).or_insert_with(|| TargetAlias {
                    triple: String::new(),
                    runner: None,
                });
                match entry.key.as_str() {
                    "triple" => alias.triple = toml_string(&entry.value)?,
                    "runner" => alias.runner = Some(toml_string(&entry.value)?),
                    _ => {}
                }
            }
            "toolchain" if sec.len() >= 2 => {
                let triple = sec[1].clone();
                let tc = pm.toolchain.entry(triple).or_default();
                match entry.key.as_str() {
                    "as" => tc.asm = Some(toml_string(&entry.value)?),
                    "ld" => tc.ld = Some(toml_string(&entry.value)?),
                    "qemu" => tc.qemu = Some(toml_string(&entry.value)?),
                    _ => {}
                }
            }
            "deploy" if sec.len() >= 2 => {
                let name = sec[1].clone();
                let dc = pm.deploy.entry(name).or_default();
                match entry.key.as_str() {
                    "format" => dc.format = Some(toml_string(&entry.value)?),
                    "sign" => dc.sign = Some(toml_bool(&entry.value)?),
                    "encrypt" => dc.encrypt = Some(toml_string(&entry.value)?),
                    "key_sign" => dc.key_sign = Some(toml_string(&entry.value)?),
                    "key_encrypt" => dc.key_encrypt = Some(toml_string(&entry.value)?),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok(pm)
}

fn process_project(proj: &mut ProjectSection, entry: &TomlEntry) {
    match entry.key.as_str() {
        "main" => {
            if let Ok(s) = toml_string(&entry.value) {
                proj.main = Some(s);
            }
        }
        "modules" => {
            if let Ok(v) = toml_string_array(&entry.value) {
                proj.modules = v;
            }
        }
        _ => {}
    }
}

fn toml_string(v: &TomlValue) -> Result<String, String> {
    match v {
        TomlValue::Str(s) => Ok(s.clone()),
        _ => Err("expected string".into()),
    }
}

fn toml_bool(v: &TomlValue) -> Result<bool, String> {
    match v {
        TomlValue::Bool(b) => Ok(*b),
        _ => Err("expected boolean".into()),
    }
}

fn toml_string_array(v: &TomlValue) -> Result<Vec<String>, String> {
    match v {
        TomlValue::StrArray(a) => Ok(a.clone()),
        TomlValue::Str(s) => Ok(vec![s.clone()]),
        _ => Err("expected string array".into()),
    }
}

/// Resolve a target name (alias or triple) to a parsed `Target`.
///  1. If `name` is a known alias in the manifest, use its triple.
///  2. Otherwise, try to parse it as a triple directly.
pub fn resolve_target(name: &str, manifest: &ProjectManifest) -> Option<codegen_core::Target> {
    let triple = if let Some(alias) = manifest.targets.get(name) {
        alias.triple.as_str()
    } else {
        name
    };
    codegen_core::Target::parse(triple.as_bytes())
}

/// Get toolchain config for a target, checking toolchain section first,
/// then falling back to manifest target aliases.
#[allow(dead_code)]
pub fn toolchain_for_target<'a>(
    target: codegen_core::Target,
    manifest: &'a ProjectManifest,
) -> Option<&'a ToolchainConfig> {
    let triple = std::str::from_utf8(target.triple()).ok()?;
    manifest.toolchain.get(triple)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_project_section() {
        let toml = r#"
[project]
main = "src/main.mod"
modules = ["src/", "sysroot/"]
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        assert_eq!(manifest.project.main.unwrap(), "src/main.mod");
        assert_eq!(manifest.project.modules, vec!["src/", "sysroot/"]);
    }

    #[test]
    fn parse_target_alias() {
        let toml = r#"
[targets.dev]
triple = "linux-x86_64-hosted"
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let alias = manifest.targets.get("dev").unwrap();
        assert_eq!(alias.triple, "linux-x86_64-hosted");
        assert!(alias.runner.is_none());
    }

    #[test]
    fn parse_toolchain_override() {
        let toml = r#"
[toolchain.armv7m-unknown-none]
as = "arm-none-eabi-as"
ld = "arm-none-eabi-ld"
qemu = "qemu-system-arm"
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let tc = manifest.toolchain.get("armv7m-unknown-none").unwrap();
        assert_eq!(tc.asm.as_deref(), Some("arm-none-eabi-as"));
        assert_eq!(tc.ld.as_deref(), Some("arm-none-eabi-ld"));
    }

    #[test]
    fn parse_deploy_section() {
        let toml = r#"
[deploy.board]
format = "lmod"
sign = true
encrypt = "device"
key_sign = "env:TYU_SIGN_KEY"
key_encrypt = "env:TYU_ENC_KEK"
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let dc = manifest.deploy.get("board").unwrap();
        assert_eq!(dc.format.as_deref(), Some("lmod"));
        assert_eq!(dc.sign, Some(true));
        assert_eq!(dc.encrypt.as_deref(), Some("device"));
    }

    #[test]
    fn resolve_target_alias() {
        let toml = r#"
[targets.board]
triple = "armv7m-unknown-none"
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let target = resolve_target("board", &manifest).unwrap();
        assert_eq!(target, codegen_core::Target::ArmV7MUnknownNone);
    }

    #[test]
    fn resolve_target_direct_triple() {
        let manifest = ProjectManifest::default();
        let target = resolve_target("x86_64-unknown-none", &manifest).unwrap();
        assert_eq!(target, codegen_core::Target::X86_64UnknownNone);
    }

    fn parse_project_manifest_from_str(text: &str) -> Result<ProjectManifest, String> {
        let entries = parse_toml(text)?;
        let mut pm = ProjectManifest::default();
        for entry in &entries {
            let sec = &entry.section;
            if sec.is_empty() { continue; }
            match sec[0].as_str() {
                "project" => process_project(&mut pm.project, entry),
                "targets" if sec.len() >= 2 => {
                    let name = sec[1].clone();
                    let alias = pm.targets.entry(name).or_insert_with(|| TargetAlias {
                        triple: String::new(), runner: None,
                    });
                    match entry.key.as_str() {
                        "triple" => alias.triple = toml_string(&entry.value).unwrap(),
                        "runner" => alias.runner = Some(toml_string(&entry.value).unwrap()),
                        _ => {}
                    }
                }
                "toolchain" if sec.len() >= 2 => {
                    let triple = sec[1].clone();
                    let tc = pm.toolchain.entry(triple).or_default();
                    match entry.key.as_str() {
                        "as" => tc.asm = Some(toml_string(&entry.value).unwrap()),
                        "ld" => tc.ld = Some(toml_string(&entry.value).unwrap()),
                        "qemu" => tc.qemu = Some(toml_string(&entry.value).unwrap()),
                        _ => {}
                    }
                }
                "deploy" if sec.len() >= 2 => {
                    let name = sec[1].clone();
                    let dc = pm.deploy.entry(name).or_default();
                    match entry.key.as_str() {
                        "format" => dc.format = Some(toml_string(&entry.value).unwrap()),
                        "sign" => dc.sign = Some(toml_bool(&entry.value).unwrap()),
                        "encrypt" => dc.encrypt = Some(toml_string(&entry.value).unwrap()),
                        "key_sign" => dc.key_sign = Some(toml_string(&entry.value).unwrap()),
                        "key_encrypt" => dc.key_encrypt = Some(toml_string(&entry.value).unwrap()),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Ok(pm)
    }
}
