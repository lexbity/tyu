//! Build artifact cache.
//!
//! Tracks compiled `.o` files keyed on `(source content hash, target triple,
//! abi_hash)`.  The cache record is persisted as `target/tyu/build.json`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Compute a 64-bit content hash for a file (FNV-1a over the bytes).
pub fn content_hash(path: &Path) -> Result<u64, String> {
    let data = fs::read(path).map_err(|e| format!("reading '{}': {}", path.display(), e))?;
    Ok(fnv1a_u64(&data))
}

/// FNV-1a 64-bit hash (deterministic, same as `lmod::hash::fnv1a_u64`).
fn fnv1a_u64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// A single artifact record in the build cache.
#[derive(Clone, Debug)]
pub struct ArtifactRecord {
    /// FNV-1a hash of the source file content at time of compilation.
    pub src_hash: u64,
    /// Target triple (e.g. `"x86_64-unknown-none"`).
    pub target: String,
    /// ABI hash at time of compilation.
    pub abi_hash: u64,
    /// Path to the output `.o` file (relative to build dir).
    pub object_path: PathBuf,
}

/// Build cache stored at `target/tyu/build.json`.
#[derive(Clone, Debug)]
pub struct BuildCache {
    /// Map from cache key to artifact record.
    /// Key format: `"{src_hash:x}-{target}-{abi_hash:x}"`.
    artifacts: HashMap<String, ArtifactRecord>,
    /// Path to the cache JSON file.
    path: PathBuf,
}

impl BuildCache {
    /// Load (or create) the build cache at the given path.
    pub fn load(path: &Path) -> Self {
        let data = fs::read_to_string(path).unwrap_or_default();
        let artifacts = if data.is_empty() {
            HashMap::new()
        } else {
            parse_json(&data).unwrap_or_default()
        };
        BuildCache {
            artifacts,
            path: path.to_path_buf(),
        }
    }

    /// Build a cache key for an artifact.
    fn key(src_hash: u64, target: &str, abi_hash: u64) -> String {
        format!("{:x}-{}-{:x}", src_hash, target, abi_hash)
    }

    /// Look up a cached artifact by source path + target + abi_hash.
    pub fn lookup(
        &self,
        src_path: &Path,
        target: &str,
        abi_hash: u64,
    ) -> Result<Option<ArtifactRecord>, String> {
        let src_hash = content_hash(src_path)?;
        let key = Self::key(src_hash, target, abi_hash);
        Ok(self.artifacts.get(&key).cloned())
    }

    /// Insert a new artifact into the cache (overwriting any existing entry
    /// for the same source+target+abi_hash).
    pub fn insert(
        &mut self,
        src_path: &Path,
        target: &str,
        abi_hash: u64,
        object_path: &Path,
    ) -> Result<(), String> {
        let src_hash = content_hash(src_path)?;
        let key = Self::key(src_hash, target, abi_hash);
        self.artifacts.insert(
            key,
            ArtifactRecord {
                src_hash,
                target: target.to_string(),
                abi_hash,
                object_path: object_path.to_path_buf(),
            },
        );
        self.save()
    }

    /// Persist the cache to disk.
    pub fn save(&self) -> Result<(), String> {
        let json = format_json(&self.artifacts);
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("creating cache dir: {}", e))?;
        }
        fs::write(&self.path, &json)
            .map_err(|e| format!("writing cache '{}': {}", self.path.display(), e))
    }
}

// ---------------------------------------------------------------------------
// Minimal JSON serialization (no serde dependency).
// ---------------------------------------------------------------------------

fn parse_json(data: &str) -> Option<HashMap<String, ArtifactRecord>> {
    let data = data.trim();
    if !data.starts_with('{') || !data.ends_with('}') {
        return None;
    }
    let inner = data[1..data.len() - 1].trim();
    if inner.is_empty() {
        return Some(HashMap::new());
    }

    let mut map = HashMap::new();
    // Split on top-level commas (not inside strings).
    let mut depth = 0i32;
    let mut start = 0usize;
    let chars: Vec<char> = inner.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                if let Some((k, v)) = parse_entry(&inner[start..i]) {
                    map.insert(k, v);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    // Last entry.
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        if let Some((k, v)) = parse_entry(tail) {
            map.insert(k, v);
        }
    }

    Some(map)
}

fn parse_entry(s: &str) -> Option<(String, ArtifactRecord)> {
    let s = s.trim();
    let colon = s.find(':')?;
    let key = s[..colon].trim().trim_matches('"').to_string();
    let val = &s[colon + 1..];

    // Parse the value object: { "src_hash": ..., "target": ..., "abi_hash": ..., "object": ... }
    let src_hash = extract_u64_field(val, "\"src_hash\"")?;
    let target = extract_string_field(val, "\"target\"")?;
    let abi_hash = extract_u64_field(val, "\"abi_hash\"")?;
    let object = extract_string_field(val, "\"object\"")?;

    Some((
        key,
        ArtifactRecord {
            src_hash,
            target,
            abi_hash,
            object_path: PathBuf::from(object),
        },
    ))
}

fn extract_u64_field(s: &str, field: &str) -> Option<u64> {
    let idx = s.find(field)?;
    let after = &s[idx + field.len()..];
    let colon = after.find(':')?;
    let start = after[colon + 1..].trim_start();
    let end = start.find(|c: char| !c.is_digit(16) && c != 'x')?;
    let num_str = &start[..end];
    let num_str = num_str.trim();
    if num_str.starts_with("0x") || num_str.starts_with("0X") {
        u64::from_str_radix(&num_str[2..], 16).ok()
    } else {
        u64::from_str_radix(num_str, 10).ok()
    }
}

fn extract_string_field<'a>(s: &str, field: &str) -> Option<String> {
    let idx = s.find(field)?;
    let after = &s[idx + field.len()..];
    let colon = after.find(':')?;
    let start = after[colon + 1..].trim_start();
    if !start.starts_with('"') {
        return None;
    }
    let end = start[1..].find('"')?;
    Some(start[1..1 + end].to_string())
}

fn format_json(artifacts: &HashMap<String, ArtifactRecord>) -> String {
    let mut s = String::from("{\n");
    let mut first = true;
    for (key, rec) in artifacts {
        if !first {
            s.push_str(",\n");
        }
        first = false;
        s.push_str(&format!(
            "  \"{}\": {{\n    \"src_hash\": {},\n    \"target\": \"{}\",\n    \"abi_hash\": {},\n    \"object\": \"{}\"\n  }}",
            key,
            rec.src_hash,
            rec.target,
            rec.abi_hash,
            rec.object_path.display(),
        ));
    }
    s.push_str("\n}\n");
    s
}
