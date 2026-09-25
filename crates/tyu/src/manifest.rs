use crate::error::TyuError;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::str::FromStr;

use harness_core::CoverageAxis;

/// What failure a poison fixture is expected to produce.
///
/// A poison fixture *must* exhibit the declared failure mode for the test
/// suite to consider it passing.  If it runs cleanly the suite reports
/// `POISON_DID_NOT_FAIL`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoisonExpectation {
    /// One or more `F` failure markers must appear in the output.
    FailMarker,
    /// The image must hang or exit without emitting `S\n`.
    NoCompletion,
    /// The image must trap with the given runtime code
    /// (e.g. `10` = stack overflow on x86_64).
    Trap(u16),
}

impl FromStr for PoisonExpectation {
    type Err = TyuError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fail-marker" => Ok(PoisonExpectation::FailMarker),
            "no-completion" => Ok(PoisonExpectation::NoCompletion),
            _ if s.starts_with("trap:") => {
                let code: u16 = s[5..]
                    .parse()
                    .map_err(|_| TyuError::Manifest(format!("invalid trap code in '{}': must be a number", s)))?;
                Ok(PoisonExpectation::Trap(code))
            }
            _ => Err(TyuError::Manifest(format!(
                "unknown poison expectation '{}': expected 'fail-marker', 'no-completion', or 'trap:N'",
                s
            ))),
        }
    }
}

impl<'de> serde::Deserialize<'de> for PoisonExpectation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// A single test fixture defined in the manifest.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FixtureEntry {
    pub name: String,
    pub file: String,
    /// Coverage axes this fixture exercises. Required and validated non-empty.
    pub axes: Vec<CoverageAxis>,
    #[serde(default)]
    pub requires: Vec<String>,
    /// Optional target triple allowlist. Empty means all targets.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Expected failure mode for a poison fixture.  `None` for normal
    /// (positive) fixtures that must pass cleanly.
    #[serde(default)]
    pub poison: Option<PoisonExpectation>,
    /// Expected number of assertions the fixture must execute.
    /// `None` (default) — no assertion-count check.
    /// `Some(n)` — the fixture must execute exactly `n` assertions;
    ///             fewer or more is a test failure.
    #[serde(default)]
    pub expects: Option<u32>,
    /// Slice 8: the verification policy this fixture's *compile* is held to.
    /// `None` (default) — the legacy `--checks=all` compile, zero behavior
    /// change. `Some(no-open)` — the fixture compiles under
    /// `--checks=undischarged` and the build fails if any obligation stays
    /// open (E6410); `no-open-no-assumptions` additionally fails on assumed
    /// verdicts. Adoption is per-suite, explicit.
    #[serde(default)]
    pub verify_policy: Option<crate::args::VerifyPolicy>,
}

/// The parsed test manifest.
#[derive(Debug, serde::Deserialize)]
struct ManifestFile {
    #[serde(rename = "fixtures", default)]
    fixtures_cfg: FixturesCfg,
    #[serde(rename = "fixture")]
    fixtures: Vec<FixtureEntry>,
}

/// Fixture-directory validation controls.
#[derive(Debug, Default, serde::Deserialize)]
pub struct FixturesCfg {
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// The parsed test manifest (public wrapper).
#[derive(Debug)]
pub struct Manifest {
    pub fixtures: Vec<FixtureEntry>,
    pub fixtures_cfg: FixturesCfg,
}

/// Parse a manifest.toml file.
pub fn parse_manifest(path: &Path) -> Result<Manifest, TyuError> {
    let text = fs::read_to_string(path)
        .map_err(|e| TyuError::Manifest(format!("reading manifest '{}': {}", path.display(), e)))?;
    let mf = toml::from_str::<ManifestFile>(&text)
        .map_err(|e| TyuError::Manifest(format!("parsing manifest '{}': {}", path.display(), e)))?;
    validate_fixture_axes(&mf.fixtures)
        .map_err(|e| TyuError::Manifest(format!("parsing manifest '{}': {}", path.display(), e)))?;
    Ok(Manifest {
        fixtures: mf.fixtures,
        fixtures_cfg: mf.fixtures_cfg,
    })
}

fn validate_fixture_axes(fixtures: &[FixtureEntry]) -> Result<(), TyuError> {
    for fixture in fixtures {
        if fixture.axes.is_empty() {
            return Err(TyuError::Manifest(format!(
                "fixture '{}' declares no axes",
                fixture.name
            )));
        }
    }
    Ok(())
}

/// Validate that the manifest and fixture directory agree exactly.
///
/// This runs before target/capability filtering so gated fixtures cannot hide
/// missing files, and unreferenced `.mod` files cannot silently rot.
pub fn validate_manifest_integrity(
    manifest: &Manifest,
    fixtures_dir: &Path,
) -> Result<(), TyuError> {
    let mut referenced_counts: BTreeMap<String, usize> = BTreeMap::new();
    for fixture in &manifest.fixtures {
        *referenced_counts.entry(fixture.file.clone()).or_insert(0) += 1;
    }

    let mut present = BTreeSet::new();
    let entries = fs::read_dir(fixtures_dir).map_err(|e| {
        TyuError::Manifest(format!(
            "reading fixtures dir '{}': {}",
            fixtures_dir.display(),
            e
        ))
    })?;
    for entry in entries {
        let entry =
            entry.map_err(|e| TyuError::Manifest(format!("reading fixtures dir entry: {}", e)))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("mod") {
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                present.insert(name.to_string());
            }
        }
    }

    let referenced: BTreeSet<String> = referenced_counts.keys().cloned().collect();
    let ignored: BTreeSet<String> = manifest.fixtures_cfg.ignore.iter().cloned().collect();

    let missing: Vec<String> = referenced.difference(&present).cloned().collect();
    let orphan: Vec<String> = present
        .difference(&referenced)
        .filter(|name| !ignored.contains(*name))
        .cloned()
        .collect();
    let duplicate: Vec<String> = referenced_counts
        .iter()
        .filter_map(|(file, count)| {
            if *count > 1 {
                Some(format!("{file} ({count} references)"))
            } else {
                None
            }
        })
        .collect();

    if missing.is_empty() && orphan.is_empty() && duplicate.is_empty() {
        return Ok(());
    }

    let mut lines = vec!["manifest drift:".to_string()];
    if !missing.is_empty() {
        lines.push(format!(
            "  missing files (referenced, absent): {}",
            missing.join(", ")
        ));
    }
    if !orphan.is_empty() {
        lines.push(format!(
            "  orphan files (present, unreferenced): {}  (add to manifest or [fixtures].ignore)",
            orphan.join(", ")
        ));
    }
    if !duplicate.is_empty() {
        lines.push(format!(
            "  duplicate file references: {}",
            duplicate.join(", ")
        ));
    }
    Err(TyuError::Manifest(lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // PoisonExpectation parsing
    // ===================================================================

    #[test]
    fn poison_fail_marker() {
        assert_eq!(
            "fail-marker".parse::<PoisonExpectation>().unwrap(),
            PoisonExpectation::FailMarker,
        );
    }

    #[test]
    fn poison_no_completion() {
        assert_eq!(
            "no-completion".parse::<PoisonExpectation>().unwrap(),
            PoisonExpectation::NoCompletion,
        );
    }

    #[test]
    fn poison_trap_code() {
        assert_eq!(
            "trap:10".parse::<PoisonExpectation>().unwrap(),
            PoisonExpectation::Trap(10),
        );
        assert_eq!(
            "trap:0".parse::<PoisonExpectation>().unwrap(),
            PoisonExpectation::Trap(0),
        );
        assert_eq!(
            "trap:5030".parse::<PoisonExpectation>().unwrap(),
            PoisonExpectation::Trap(5030),
        );
    }

    #[test]
    fn poison_trap_invalid_code() {
        assert!("trap:abc".parse::<PoisonExpectation>().is_err());
        assert!("trap:".parse::<PoisonExpectation>().is_err());
        assert!("trap:-1".parse::<PoisonExpectation>().is_err());
    }

    #[test]
    fn poison_unknown_string() {
        assert!("unknown".parse::<PoisonExpectation>().is_err());
        assert!("".parse::<PoisonExpectation>().is_err());
        assert!("  fail-marker  ".parse::<PoisonExpectation>().is_err());
    }

    // ===================================================================
    // FixtureEntry deserialization
    // ===================================================================

    #[test]
    fn parse_simple_manifest() {
        let toml = r#"
[[fixture]]
name = "arithmetic"
file = "arithmetic.mod"
axes = ["arith", "controlflow"]
requires = []

[[fixture]]
name = "stack_ops"
file = "stack_ops.mod"
axes = ["stack"]
requires = []
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert!(mf.fixtures_cfg.ignore.is_empty());
        assert_eq!(mf.fixtures.len(), 2);
        assert_eq!(mf.fixtures[0].name, "arithmetic");
        assert_eq!(mf.fixtures[0].file, "arithmetic.mod");
        assert_eq!(
            mf.fixtures[0].axes,
            vec![CoverageAxis::Arith, CoverageAxis::Controlflow]
        );
        assert!(mf.fixtures[0].requires.is_empty());
        assert!(mf.fixtures[0].poison.is_none());
    }

    #[test]
    fn parse_manifest_rejects_empty_axes() {
        let dir = temp_fixtures_dir("empty_axes_parse");
        let path = dir.join("manifest.toml");
        fs::write(
            &path,
            r#"
[[fixture]]
name = "test"
file = "test.mod"
axes = []
requires = []
"#,
        )
        .unwrap();
        let err = parse_manifest(&path).unwrap_err();
        assert!(
            err.to_string().contains("fixture 'test' declares no axes"),
            "{err}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_manifest_rejects_absent_axes() {
        let dir = temp_fixtures_dir("absent_axes_parse");
        let path = dir.join("manifest.toml");
        fs::write(
            &path,
            r#"
[[fixture]]
name = "test"
file = "test.mod"
requires = []
"#,
        )
        .unwrap();
        let err = parse_manifest(&path).unwrap_err();
        assert!(err.to_string().contains("missing field `axes`"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_manifest_rejects_unknown_axis() {
        let toml = r#"
[[fixture]]
name = "test"
file = "test.mod"
axes = ["not-an-axis"]
requires = []
"#;
        assert!(toml::from_str::<ManifestFile>(toml).is_err());
    }

    #[test]
    fn parse_poison_manifest() {
        let toml = r#"
[[fixture]]
name = "poison_stack"
file = "poison_stack.mod"
axes = ["trap"]
requires = []
poison = "trap:10"

[[fixture]]
name = "poison_fail"
file = "poison_fail.mod"
axes = ["trap"]
requires = []
poison = "fail-marker"

[[fixture]]
name = "poison_nocomp"
file = "poison_nocomp.mod"
axes = ["trap"]
requires = []
poison = "no-completion"

[[fixture]]
name = "normal"
file = "normal.mod"
axes = ["arith"]
requires = []
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures.len(), 4);

        assert_eq!(mf.fixtures[0].name, "poison_stack");
        assert_eq!(mf.fixtures[0].poison, Some(PoisonExpectation::Trap(10)));

        assert_eq!(mf.fixtures[1].name, "poison_fail");
        assert_eq!(mf.fixtures[1].poison, Some(PoisonExpectation::FailMarker));

        assert_eq!(mf.fixtures[2].name, "poison_nocomp");
        assert_eq!(mf.fixtures[2].poison, Some(PoisonExpectation::NoCompletion));

        assert_eq!(mf.fixtures[3].name, "normal");
        assert_eq!(mf.fixtures[3].poison, None);
    }

    #[test]
    fn parse_expects_field() {
        let toml = r#"
[[fixture]]
name = "test"
file = "test.mod"
axes = ["arith"]
requires = []
expects = 5
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].expects, Some(5));
    }

    #[test]
    fn parse_expects_defaults_to_none() {
        let toml = r#"
[[fixture]]
name = "test"
file = "test.mod"
axes = ["arith"]
requires = []
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].expects, None);
    }

    #[test]
    fn parse_fixtures_ignore_defaults_to_empty() {
        let toml = r#"
[[fixture]]
name = "test"
file = "test.mod"
axes = ["arith"]
requires = []
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert!(mf.fixtures_cfg.ignore.is_empty());
    }

    #[test]
    fn parse_fixtures_ignore_list() {
        let toml = r#"
[fixtures]
ignore = ["scratch.mod"]

[[fixture]]
name = "test"
file = "test.mod"
axes = ["arith"]
requires = []
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures_cfg.ignore, vec!["scratch.mod"]);
    }

    #[test]
    fn parse_poison_rejects_invalid() {
        let toml = r#"
[[fixture]]
name = "bad"
file = "bad.mod"
axes = ["arith"]
requires = []
poison = "splat"
"#;
        assert!(toml::from_str::<ManifestFile>(toml).is_err());
    }

    // ===================================================================
    // Existing manifest parsing tests (unchanged)
    // ===================================================================

    #[test]
    fn parse_capability_requires() {
        let toml = r#"
[[fixture]]
name = "channels"
file = "channels.mod"
axes = ["concurrency"]
requires = ["Channels"]

[[fixture]]
name = "tasks"
file = "tasks.mod"
axes = ["concurrency"]
requires = ["TaskScheduler"]
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].requires, vec!["Channels"]);
        assert_eq!(mf.fixtures[1].requires, vec!["TaskScheduler"]);
    }

    fn temp_fixtures_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tyu_manifest_integrity_{}_{}",
            label,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manifest(files: &[&str], ignore: &[&str]) -> Manifest {
        Manifest {
            fixtures: files
                .iter()
                .enumerate()
                .map(|(i, file)| FixtureEntry {
                    name: format!("fixture_{i}"),
                    file: (*file).to_string(),
                    axes: vec![CoverageAxis::Arith],
                    requires: Vec::new(),
                    targets: Vec::new(),
                    poison: None,
                    expects: None,
                    verify_policy: None,
                })
                .collect(),
            fixtures_cfg: FixturesCfg {
                ignore: ignore.iter().map(|s| (*s).to_string()).collect(),
            },
        }
    }

    #[test]
    fn integrity_missing_file_errs() {
        let dir = temp_fixtures_dir("missing");
        fs::write(dir.join("present.mod"), "module Present; end;\n").unwrap();
        let err =
            validate_manifest_integrity(&manifest(&["present.mod", "missing.mod"], &[]), &dir)
                .unwrap_err();
        assert!(err.to_string().contains("missing files"));
        assert!(err.to_string().contains("missing.mod"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn integrity_orphan_errs() {
        let dir = temp_fixtures_dir("orphan");
        fs::write(dir.join("present.mod"), "module Present; end;\n").unwrap();
        fs::write(dir.join("orphan.mod"), "module Orphan; end;\n").unwrap();
        let err = validate_manifest_integrity(&manifest(&["present.mod"], &[]), &dir).unwrap_err();
        assert!(err.to_string().contains("orphan files"));
        assert!(err.to_string().contains("orphan.mod"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn integrity_ignore_allows_orphan() {
        let dir = temp_fixtures_dir("ignore");
        fs::write(dir.join("present.mod"), "module Present; end;\n").unwrap();
        fs::write(dir.join("scratch.mod"), "module Scratch; end;\n").unwrap();
        validate_manifest_integrity(&manifest(&["present.mod"], &["scratch.mod"]), &dir).unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn integrity_clean_ok() {
        let dir = temp_fixtures_dir("clean");
        fs::write(dir.join("a.mod"), "module A; end;\n").unwrap();
        fs::write(dir.join("b.mod"), "module B; end;\n").unwrap();
        validate_manifest_integrity(&manifest(&["a.mod", "b.mod"], &[]), &dir).unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn integrity_duplicate_reference_errs() {
        let dir = temp_fixtures_dir("duplicate");
        fs::write(dir.join("a.mod"), "module A; end;\n").unwrap();
        let err =
            validate_manifest_integrity(&manifest(&["a.mod", "a.mod"], &[]), &dir).unwrap_err();
        assert!(err.to_string().contains("duplicate file references"));
        assert!(err.to_string().contains("a.mod"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_manifest() {
        let toml = "";
        let result: Result<ManifestFile, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn file_before_name() {
        let toml = r#"
[[fixture]]
file = "test.mod"
name = "test"
axes = ["arith"]
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures.len(), 1);
        assert_eq!(mf.fixtures[0].name, "test");
        assert_eq!(mf.fixtures[0].file, "test.mod");
    }

    #[test]
    fn multiple_fixtures_interleaved_keys() {
        let toml = r#"
[[fixture]]
name = "a"
file = "a.mod"
axes = ["arith"]

[[fixture]]
name = "b"
file = "b.mod"
axes = ["stack"]
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures.len(), 2);
        assert_eq!(mf.fixtures[0].name, "a");
        assert_eq!(mf.fixtures[1].name, "b");
    }

    #[test]
    fn comment_after_value_ignored() {
        let toml = r#"
[[fixture]]
name = "test"
file = "test.mod" # this is a comment
axes = ["arith"]
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].name, "test");
    }

    #[test]
    fn quoted_hash_in_value() {
        let toml = r#"
[[fixture]]
name = "c#1"
file = "c#1.mod"
axes = ["arith"]
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].name, "c#1");
        assert_eq!(mf.fixtures[0].file, "c#1.mod");
    }
}
