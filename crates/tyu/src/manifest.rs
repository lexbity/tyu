use std::fs;
use std::path::Path;
use std::str::FromStr;

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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fail-marker" => Ok(PoisonExpectation::FailMarker),
            "no-completion" => Ok(PoisonExpectation::NoCompletion),
            _ if s.starts_with("trap:") => {
                let code: u16 = s[5..]
                    .parse()
                    .map_err(|_| format!("invalid trap code in '{}': must be a number", s))?;
                Ok(PoisonExpectation::Trap(code))
            }
            _ => Err(format!(
                "unknown poison expectation '{}': expected 'fail-marker', 'no-completion', or 'trap:N'",
                s
            )),
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
    #[serde(default)]
    pub requires: Vec<String>,
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
}

/// The parsed test manifest.
#[derive(Debug, serde::Deserialize)]
struct ManifestFile {
    #[serde(rename = "fixture")]
    fixtures: Vec<FixtureEntry>,
}

/// The parsed test manifest (public wrapper).
#[derive(Debug)]
pub struct Manifest {
    pub fixtures: Vec<FixtureEntry>,
}

/// Parse a manifest.toml file.
pub fn parse_manifest(path: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("reading manifest '{}': {}", path.display(), e))?;
    toml::from_str::<ManifestFile>(&text)
        .map(|mf| Manifest {
            fixtures: mf.fixtures,
        })
        .map_err(|e| format!("parsing manifest '{}': {}", path.display(), e))
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
requires = []

[[fixture]]
name = "stack_ops"
file = "stack_ops.mod"
requires = []
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures.len(), 2);
        assert_eq!(mf.fixtures[0].name, "arithmetic");
        assert_eq!(mf.fixtures[0].file, "arithmetic.mod");
        assert!(mf.fixtures[0].requires.is_empty());
        assert!(mf.fixtures[0].poison.is_none());
    }

    #[test]
    fn parse_poison_manifest() {
        let toml = r#"
[[fixture]]
name = "poison_stack"
file = "poison_stack.mod"
requires = []
poison = "trap:10"

[[fixture]]
name = "poison_fail"
file = "poison_fail.mod"
requires = []
poison = "fail-marker"

[[fixture]]
name = "poison_nocomp"
file = "poison_nocomp.mod"
requires = []
poison = "no-completion"

[[fixture]]
name = "normal"
file = "normal.mod"
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
requires = []
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].expects, None);
    }

    #[test]
    fn parse_poison_rejects_invalid() {
        let toml = r#"
[[fixture]]
name = "bad"
file = "bad.mod"
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
requires = ["Channels"]

[[fixture]]
name = "tasks"
file = "tasks.mod"
requires = ["TaskScheduler"]
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].requires, vec!["Channels"]);
        assert_eq!(mf.fixtures[1].requires, vec!["TaskScheduler"]);
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

[[fixture]]
name = "b"
file = "b.mod"
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
"#;
        let mf: ManifestFile = toml::from_str(toml).unwrap();
        assert_eq!(mf.fixtures[0].name, "c#1");
        assert_eq!(mf.fixtures[0].file, "c#1.mod");
    }
}
