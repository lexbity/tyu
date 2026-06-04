//! Test-manifest parser.
//!
//! Reads `manifest.toml` from the fixtures directory to discover test suites,
//! their source files, and capability requirements.

use std::fs;
use std::path::Path;

use crate::toml_parser::{parse_toml, TomlValue};

/// A single test fixture defined in the manifest.
#[derive(Clone, Debug)]
pub struct FixtureEntry {
    pub name: String,
    pub file: String,
    pub requires: Vec<String>,
}

/// The parsed test manifest.
#[derive(Debug)]
pub struct Manifest {
    pub fixtures: Vec<FixtureEntry>,
}

/// Parse a manifest.toml file.
pub fn parse_manifest(path: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("reading manifest '{}': {}", path.display(), e))?;
    parse_manifest_str(&text)
}

fn parse_manifest_str(text: &str) -> Result<Manifest, String> {
    let entries = parse_toml(text)?;
    let mut fixtures: Vec<FixtureEntry> = Vec::new();
    // Group entries by array-of-tables index.
    // For [[fixture]], all consecutive entries with section=["fixture"] belong
    // to the same fixture until a gap (new [[fixture]]).
    let mut current_name: Option<String> = None;
    let mut current_file: Option<String> = None;
    let mut current_requires: Vec<String> = Vec::new();

    for entry in &entries {
        if entry.section != vec!["fixture"] {
            continue;
        }
        match entry.key.as_str() {
            "name" => {
                flush_fixture(&mut fixtures, &mut current_name, &mut current_file, &mut current_requires);
                current_name = Some(toml_string(&entry.value)?);
            }
            "file" => {
                current_file = Some(toml_string(&entry.value)?);
            }
            "requires" => {
                current_requires = toml_string_array(&entry.value)?;
            }
            _ => {}
        }
    }
    flush_fixture(&mut fixtures, &mut current_name, &mut current_file, &mut current_requires);

    Ok(Manifest { fixtures })
}

fn flush_fixture(
    fixtures: &mut Vec<FixtureEntry>,
    name: &mut Option<String>,
    file: &mut Option<String>,
    requires: &mut Vec<String>,
) {
    if let (Some(n), Some(f)) = (name.take(), file.take()) {
        fixtures.push(FixtureEntry {
            name: n,
            file: f,
            requires: std::mem::take(requires),
        });
    }
}

fn toml_string(v: &TomlValue) -> Result<String, String> {
    match v {
        TomlValue::Str(s) => Ok(s.clone()),
        _ => Err("expected string".into()),
    }
}

fn toml_string_array(v: &TomlValue) -> Result<Vec<String>, String> {
    match v {
        TomlValue::StrArray(a) => Ok(a.clone()),
        TomlValue::Str(_) => Ok(vec![toml_string(v)?]),
        _ => Err("expected string array".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let m = parse_manifest_str(toml).unwrap();
        assert_eq!(m.fixtures.len(), 2);
        assert_eq!(m.fixtures[0].name, "arithmetic");
        assert_eq!(m.fixtures[0].file, "arithmetic.mod");
        assert!(m.fixtures[0].requires.is_empty());
    }

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
        let m = parse_manifest_str(toml).unwrap();
        assert_eq!(m.fixtures[0].requires, vec!["Channels"]);
        assert_eq!(m.fixtures[1].requires, vec!["TaskScheduler"]);
    }

    #[test]
    fn empty_manifest() {
        let m = parse_manifest_str("").unwrap();
        assert!(m.fixtures.is_empty());
    }
}
