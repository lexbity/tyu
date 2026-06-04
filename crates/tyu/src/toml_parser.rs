//! Minimal TOML parser for `tyu.toml` and `manifest.toml`.
//!
//! Handles the subset of TOML used by this project:
//! - `[section]` / `[section.subsection]` headers
//! - `[[array-of-tables]]` headers
//! - `key = "string"`, `key = true`, `key = false`
//! - `key = ["val1", "val2"]` string arrays
//! - `# comments`
//!
//! Produces a flat list of `(section_path, key, value)` entries.

/// A parsed TOML value.
#[derive(Clone, Debug, PartialEq)]
pub enum TomlValue {
    Str(String),
    Bool(bool),
    StrArray(Vec<String>),
}

/// A single parsed entry: section path + key + value.
#[derive(Clone, Debug)]
pub struct TomlEntry {
    pub section: Vec<String>,
    pub key: String,
    pub value: TomlValue,
}

/// Parse TOML text into a list of entries.
pub fn parse_toml(text: &str) -> Result<Vec<TomlEntry>, String> {
    let mut entries: Vec<TomlEntry> = Vec::new();
    let mut current_section: Vec<String> = Vec::new();

    for (lineno, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Section header: [section] or [section.subsection]
        if line.starts_with('[') {
            let inner = line.trim_end_matches(']').trim_start_matches('[').trim();
            if line.starts_with("[[") {
                // [[array-of-tables]]
                current_section = parse_section_path(inner)?;
                // Array-of-tables entries are key-value pairs under the section.
                // The section itself will be used by subsequent key=value lines.
            } else {
                // [section] or [section.subsection]
                current_section = parse_section_path(inner)?;
            }
            continue;
        }

        // Key = value
        let eq_pos = line.find('=').ok_or_else(|| {
            format!("line {}: expected key=value", lineno + 1)
        })?;
        let key = line[..eq_pos].trim().to_string();
        let val_str = line[eq_pos + 1..].trim();

        if key.is_empty() {
            return Err(format!("line {}: empty key", lineno + 1));
        }

        let value = parse_value(val_str, lineno)?;
        entries.push(TomlEntry {
            section: current_section.clone(),
            key,
            value,
        });
    }

    Ok(entries)
}

fn parse_section_path(s: &str) -> Result<Vec<String>, String> {
    Ok(s.split('.').map(|p| p.trim().to_string()).collect())
}

fn parse_value(s: &str, lineno: usize) -> Result<TomlValue, String> {
    let s = s.trim();
    if s.starts_with('"') {
        // String: "value"
        if !s.ends_with('"') {
            return Err(format!("line {}: unclosed string", lineno + 1));
        }
        Ok(TomlValue::Str(s[1..s.len() - 1].to_string()))
    } else if s.starts_with('[') {
        // Array: ["val1", "val2"]
        let inner = s.trim_end_matches(']').trim_start_matches('[').trim();
        if inner.is_empty() {
            return Ok(TomlValue::StrArray(Vec::new()));
        }
        let mut items = Vec::new();
        // Split by comma, respecting quoted strings.
        let mut i = 0;
        let bytes = inner.as_bytes();
        while i < bytes.len() {
            // Skip whitespace.
            while i < bytes.len() && bytes[i] == b' ' { i += 1; }
            if i >= bytes.len() { break; }
            if bytes[i] == b'"' {
                // Quoted string
                let start = i + 1;
                i = start;
                while i < bytes.len() && bytes[i] != b'"' { i += 1; }
                if i >= bytes.len() {
                    return Err(format!("line {}: unclosed string in array", lineno + 1));
                }
                items.push(inner[start..i].to_string());
                i += 1; // skip closing "
                // Skip to comma or end
                while i < bytes.len() && bytes[i] != b',' { i += 1; }
                i += 1; // skip comma
            } else {
                // Unquoted item — skip to comma (shouldn't happen in our format)
                while i < bytes.len() && bytes[i] != b',' { i += 1; }
                i += 1;
            }
        }
        Ok(TomlValue::StrArray(items))
    } else if s == "true" {
        Ok(TomlValue::Bool(true))
    } else if s == "false" {
        Ok(TomlValue::Bool(false))
    } else {
        Err(format!("line {}: unexpected value '{}'", lineno + 1, s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_section() {
        let toml = r#"
[project]
main = "src/main.mod"
"#;
        let entries = parse_toml(toml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].section, vec!["project"]);
        assert_eq!(entries[0].key, "main");
        assert_eq!(entries[0].value, TomlValue::Str("src/main.mod".into()));
    }

    #[test]
    fn parse_nested_section() {
        let toml = r#"
[targets.dev]
triple = "linux-x86_64-hosted"
"#;
        let entries = parse_toml(toml).unwrap();
        assert_eq!(entries[0].section, vec!["targets", "dev"]);
    }

    #[test]
    fn parse_array() {
        let toml = r#"
[project]
modules = ["src/", "sysroot/"]
"#;
        let entries = parse_toml(toml).unwrap();
        assert_eq!(entries[0].value, TomlValue::StrArray(vec!["src/".into(), "sysroot/".into()]));
    }

    #[test]
    fn parse_bool() {
        let toml = r#"
[deploy.board]
sign = true
"#;
        let entries = parse_toml(toml).unwrap();
        assert_eq!(entries[0].value, TomlValue::Bool(true));
    }

    #[test]
    fn parse_array_of_tables() {
        let toml = r#"
[[fixture]]
name = "arithmetic"
file = "arithmetic.mod"
"#;
        let entries = parse_toml(toml).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].section, vec!["fixture"]);
        assert_eq!(entries[0].key, "name");
    }

    #[test]
    fn empty_input() {
        let entries = parse_toml("").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn comments_ignored() {
        let toml = "# this is a comment\n[project]\n# another comment\nmain = \"x\"\n";
        let entries = parse_toml(toml).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn boolean_false() {
        let toml = "[x]\ny = false\n";
        let entries = parse_toml(toml).unwrap();
        assert_eq!(entries[0].value, TomlValue::Bool(false));
    }
}
