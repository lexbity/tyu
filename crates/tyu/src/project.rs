use crate::error::TyuError;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use codegen_core::{Feature, FeatureSet};

/// `[project]` section.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct ProjectSection {
    pub main: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
}

/// `[targets.<name>]` — a named target alias.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct TargetAlias {
    pub triple: String,
    #[serde(default)]
    pub runner: Option<String>,
}

/// `[toolchain.<triple>]` — tool overrides for a target triple.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct ToolchainConfig {
    #[serde(default, rename = "as")]
    pub asm: Option<String>,
    #[serde(default)]
    pub ld: Option<String>,
    #[serde(default)]
    pub qemu: Option<String>,
}

/// `[deploy.<name>]` — deployment configuration.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct DeployConfig {
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub sign: Option<bool>,
    #[serde(default)]
    pub encrypt: Option<String>,
    #[serde(default)]
    pub key_sign: Option<String>,
    #[serde(default)]
    pub key_encrypt: Option<String>,
}

/// `[profile.<name>]` — image-level build profile.
///
/// A profile selects a set of features that control which language constructs
/// are accepted and which runtime units are linked into the final image.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct ProfileConfig {
    /// Image features enabled in this profile.
    /// Recognised values: `"concurrency"`, `"module-loading"`.
    #[serde(default)]
    pub features: Vec<String>,
}

/// The complete project manifest.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct ProjectManifest {
    #[serde(default)]
    pub project: ProjectSection,
    #[serde(default)]
    pub targets: HashMap<String, TargetAlias>,
    #[serde(default)]
    pub toolchain: HashMap<String, ToolchainConfig>,
    #[serde(default)]
    pub deploy: HashMap<String, DeployConfig>,
    /// Per-profile feature sets.  The active profile is selected by
    /// `tyu --profile=<name>` (default: `"dev"` if present, else all-features-on).
    #[serde(default)]
    pub profile: HashMap<String, ProfileConfig>,
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
pub fn parse_project_manifest(path: &Path) -> Result<ProjectManifest, TyuError> {
    let text = fs::read_to_string(path)
        .map_err(|e| TyuError::Project(format!("reading '{}': {}", path.display(), e)))?;
    toml::from_str(&text)
        .map_err(|e| TyuError::Project(format!("parsing '{}': {}", path.display(), e)))
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

/// Resolve a profile name (or the implicit default) to a [`FeatureSet`].
///
/// Resolution rules (per Q3):
///  1. If `profile_name_opt` is `Some(n)`, look up profile `n` — error if missing.
///  2. If `None` and manifest has a `"dev"` profile, use it.
///  3. Otherwise, return `FeatureSet::all()` (implicit all-features-on default).
///
/// Returns `(feature_set, resolved_profile_name)`.
pub fn resolve_feature_set(
    profile_name_opt: Option<&str>,
    manifest: &ProjectManifest,
) -> Result<(FeatureSet, Option<String>), TyuError> {
    let name = profile_name_opt.map(|n| n.to_string()).or_else(|| {
        if manifest.profile.contains_key("dev") {
            Some("dev".to_string())
        } else {
            None
        }
    });

    let set = match name {
        Some(ref n) => {
            let pc = manifest
                .profile
                .get(n.as_str())
                .ok_or_else(|| TyuError::Project(format!("unknown profile '{}'", n)))?;
            let mut set = FeatureSet::empty();
            for f_str in &pc.features {
                let f = Feature::parse(f_str).ok_or_else(|| {
                    TyuError::Project(format!("unknown feature '{}' in profile '{}'", f_str, n))
                })?;
                set = set.with(f);
            }
            set
        }
        None => FeatureSet::all(),
    };

    Ok((set, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_project_manifest_from_str(text: &str) -> Result<ProjectManifest, TyuError> {
        toml::from_str(text).map_err(|e| TyuError::Project(e.to_string()))
    }

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

    #[test]
    fn default_manifest_empty() {
        let pm = ProjectManifest::default();
        assert!(pm.project.main.is_none());
        assert!(pm.project.modules.is_empty());
        assert!(pm.targets.is_empty());
        assert!(pm.toolchain.is_empty());
        assert!(pm.deploy.is_empty());
    }

    #[test]
    fn targets_are_optional() {
        let toml = r#"
[project]
main = "main.mod"
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        assert_eq!(manifest.project.main.unwrap(), "main.mod");
        assert!(manifest.targets.is_empty());
    }

    // -----------------------------------------------------------------------
    // resolve_feature_set
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_feature_set_implicit_default_all() {
        let manifest = ProjectManifest::default();
        let (set, name) = resolve_feature_set(None, &manifest).unwrap();
        assert!(set.contains(codegen_core::Feature::Concurrency));
        assert!(set.contains(codegen_core::Feature::ModuleLoading));
        assert!(name.is_none());
    }

    #[test]
    fn resolve_feature_set_dev_profile() {
        let toml = r#"
[profile.dev]
features = ["concurrency", "module-loading"]
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let (set, name) = resolve_feature_set(Some("dev"), &manifest).unwrap();
        assert!(set.contains(codegen_core::Feature::Concurrency));
        assert!(set.contains(codegen_core::Feature::ModuleLoading));
        assert_eq!(name.as_deref(), Some("dev"));
    }

    #[test]
    fn resolve_feature_set_slim_profile() {
        let toml = r#"
[profile.slim]
features = ["concurrency"]
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let (set, _) = resolve_feature_set(Some("slim"), &manifest).unwrap();
        assert!(set.contains(codegen_core::Feature::Concurrency));
        assert!(!set.contains(codegen_core::Feature::ModuleLoading));
    }

    #[test]
    fn resolve_feature_set_empty_features() {
        let toml = r#"
[profile.minimal]
features = []
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let (set, _) = resolve_feature_set(Some("minimal"), &manifest).unwrap();
        assert!(!set.contains(codegen_core::Feature::Concurrency));
        assert!(!set.contains(codegen_core::Feature::ModuleLoading));
    }

    #[test]
    fn resolve_feature_set_unknown_profile_errs() {
        let manifest = ProjectManifest::default();
        let err = resolve_feature_set(Some("nonexistent"), &manifest).unwrap_err();
        assert!(err.to_string().contains("unknown profile"), "error: {err}");
    }

    #[test]
    fn resolve_feature_set_unknown_feature_errs() {
        let toml = r#"
[profile.x]
features = ["bogus"]
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let err = resolve_feature_set(Some("x"), &manifest).unwrap_err();
        assert!(err.to_string().contains("unknown feature"), "error: {err}");
    }

    #[test]
    fn resolve_feature_set_dev_implicit_when_present() {
        let toml = r#"
[profile.dev]
features = ["concurrency"]
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        let (set, name) = resolve_feature_set(None, &manifest).unwrap();
        assert!(set.contains(codegen_core::Feature::Concurrency));
        assert!(!set.contains(codegen_core::Feature::ModuleLoading));
        assert_eq!(name.as_deref(), Some("dev"));
    }

    #[test]
    fn resolve_feature_set_keeps_unrecognized_fields() {
        // Extra fields in profile should be tolerated (serde default).
        let toml = r#"
[profile.foo]
features = ["concurrency"]
extra_field = true
"#;
        let manifest = parse_project_manifest_from_str(toml).unwrap();
        assert!(manifest.profile.contains_key("foo"));
    }
}
