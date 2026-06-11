use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use codegen_core::CODEGEN_REV;

use crate::error::TyuError;

/// Compute a 64-bit content hash for a file (FNV-1a over the bytes).
pub fn content_hash(path: &Path) -> Result<u64, TyuError> {
    let data = fs::read(path).map_err(TyuError::Io)?;
    Ok(fnv1a_u64(&data))
}

/// FNV-1a 64-bit hash (deterministic, same as `lmod::hash::fnv1a_u64`).
pub(crate) fn fnv1a_u64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Produce a compiler fingerprint: folds `CODEGEN_REV` and the `langc` binary
/// mtime into a single u64.  When either changes, all cache keys change.
pub fn compiler_fingerprint() -> u64 {
    let mut h = fnv1a_u64(&CODEGEN_REV.to_le_bytes());
    // Fold in the langc binary mtime if we can find it.
    if let Ok(langc) = crate::toolchain::resolve_tool("langc") {
        if let Ok(meta) = fs::metadata(&langc) {
            if let Ok(mtime) = meta.modified() {
                let nanos = mtime.duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                h = h.wrapping_mul(0x100000001b3);
                h ^= fnv1a_u64(&nanos.to_le_bytes());
            }
        }
    }
    h
}

/// Compute the input-set fingerprint for a module.
///
/// `inputs_fp` = FNV-1a over the concatenation of:
///   - own source content hash (8 bytes LE)
///   - target triple bytes
///   - sorted transitive dependency content hashes (each 8 bytes LE)
pub fn inputs_fingerprint(
    own_hash: u64,
    triple: &str,
    transitive_dep_hashes: &[u64],
) -> u64 {
    let mut buf = Vec::with_capacity(8 + triple.len() + transitive_dep_hashes.len() * 8);
    buf.extend_from_slice(&own_hash.to_le_bytes());
    buf.extend_from_slice(triple.as_bytes());
    let mut sorted = transitive_dep_hashes.to_vec();
    sorted.sort();
    for &h in &sorted {
        buf.extend_from_slice(&h.to_le_bytes());
    }
    fnv1a_u64(&buf)
}

/// Collect the content hashes of all transitive dependencies of `module`
/// (recursively through `dep_paths`), deduplicated and sorted.
/// Caches results in `cache` to avoid recomputation across modules.
pub fn collect_transitive_hashes(
    module: &crate::graph::ModuleNode,
    path_to_hash: &BTreeMap<PathBuf, u64>,
    transitive_cache: &mut BTreeMap<PathBuf, Vec<u64>>,
) -> Vec<u64> {
    let mod_path = &module.path;
    if let Some(cached) = transitive_cache.get(mod_path) {
        return cached.clone();
    }

    let mut result: Vec<u64> = Vec::new();
    for dep_path in &module.dep_paths {
        if let Some(&h) = path_to_hash.get(dep_path) {
            result.push(h);
        }
        if let Some(sub) = transitive_cache.get(dep_path) {
            result.extend(sub);
        }
    }
    result.sort();
    result.dedup();

    transitive_cache.insert(mod_path.clone(), result.clone());
    result
}

/// A single artifact record in the build cache.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ArtifactRecord {
    /// Compiler fingerprint at time of compilation.
    pub compiler_fp: u64,
    /// Input-set fingerprint (own source + transitive deps + target).
    pub inputs_fp: u64,
    /// ABI hash at time of compilation.
    pub abi_hash: u64,
    /// Target triple.
    pub target: String,
    /// Path to the output `.o` file.
    pub object_path: PathBuf,
}

/// On-disk cache file layout.
#[derive(serde::Serialize, serde::Deserialize)]
struct CacheFile {
    version: u32,
    artifacts: BTreeMap<String, ArtifactRecord>,
}

/// Build cache stored at `target/tyu/build.json`.
#[derive(Clone, Debug)]
pub struct BuildCache {
    /// Map from cache key to artifact record.
    artifacts: BTreeMap<String, ArtifactRecord>,
    /// Path to the cache JSON file.
    path: PathBuf,
}

impl BuildCache {
    /// Load (or create) the build cache at the given path.
    /// Unreadable or wrong-version files are silently treated as empty.
    pub fn load(path: &Path) -> Self {
        let artifacts = fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str::<CacheFile>(&data).ok())
            .filter(|cf| cf.version == 2)
            .map(|cf| cf.artifacts)
            .unwrap_or_default();
        BuildCache {
            artifacts,
            path: path.to_path_buf(),
        }
    }

    /// Build a cache key.
    fn key(compiler_fp: u64, inputs_fp: u64, abi_hash: u64) -> String {
        format!("{:x}-{:x}-{:x}", compiler_fp, inputs_fp, abi_hash)
    }

    /// Look up a cached artifact.
    pub fn lookup(
        &self,
        compiler_fp: u64,
        inputs_fp: u64,
        abi_hash: u64,
    ) -> Option<ArtifactRecord> {
        let key = Self::key(compiler_fp, inputs_fp, abi_hash);
        self.artifacts.get(&key).cloned()
    }

    /// Insert a new artifact into the cache.
    pub fn insert(
        &mut self,
        compiler_fp: u64,
        inputs_fp: u64,
        abi_hash: u64,
        target: &str,
        object_path: &Path,
    ) -> Result<(), TyuError> {
        let key = Self::key(compiler_fp, inputs_fp, abi_hash);
        self.artifacts.insert(
            key,
            ArtifactRecord {
                compiler_fp,
                inputs_fp,
                abi_hash,
                target: target.to_string(),
                object_path: object_path.to_path_buf(),
            },
        );
        self.save()
    }

    /// Persist the cache to disk.
    pub fn save(&self) -> Result<(), TyuError> {
        let cf = CacheFile {
            version: 2,
            artifacts: self.artifacts.clone(),
        };
        let json = serde_json::to_string_pretty(&cf)
            .map_err(|e| TyuError::Cache(format!("serializing: {}", e)))?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(TyuError::Io)?;
        }
        fs::write(&self.path, &json).map_err(TyuError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("tyu_cache_tests")
            .join(format!("{}_{}", label, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_cache(path: &Path) -> BuildCache {
        BuildCache::load(path)
    }

    #[test]
    fn content_hash_is_deterministic() {
        let dir = temp_dir("hash_det");
        let p = dir.join("t.txt");
        fs::write(&p, b"hello world").unwrap();
        assert_eq!(content_hash(&p).unwrap(), content_hash(&p).unwrap());
    }

    #[test]
    fn different_content_different_hash() {
        let dir = temp_dir("hash_diff");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, b"hello").unwrap();
        fs::write(&b, b"world").unwrap();
        assert_ne!(content_hash(&a).unwrap(), content_hash(&b).unwrap());
    }

    #[test]
    fn lookup_miss_on_unknown_key() {
        let dir = temp_dir("lookup_miss");
        let c = make_cache(&dir.join("build.json"));
        assert!(c.lookup(0, 0, 0).is_none());
    }

    #[test]
    fn insert_then_lookup_hit() {
        let dir = temp_dir("insert_lookup");
        let p = dir.join("build.json");
        let mut c = make_cache(&p);
        let obj = dir.join("out.o");
        fs::write(&obj, b"\x7fELF").unwrap();
        c.insert(1, 2, 3, "test", &obj).unwrap();
        let r = c.lookup(1, 2, 3).unwrap();
        assert_eq!(r.object_path, obj);
    }

    #[test]
    fn lookup_miss_on_different_compiler_fp() {
        let dir = temp_dir("compiler_miss");
        let p = dir.join("build.json");
        let mut c = make_cache(&p);
        let obj = dir.join("out.o");
        fs::write(&obj, b"\x7fELF").unwrap();
        c.insert(1, 2, 3, "test", &obj).unwrap();
        assert!(c.lookup(99, 2, 3).is_none(), "different compiler_fp must miss");
    }

    #[test]
    fn lookup_miss_on_different_inputs_fp() {
        let dir = temp_dir("inputs_miss");
        let p = dir.join("build.json");
        let mut c = make_cache(&p);
        let obj = dir.join("out.o");
        fs::write(&obj, b"\x7fELF").unwrap();
        c.insert(1, 2, 3, "test", &obj).unwrap();
        assert!(c.lookup(1, 99, 3).is_none(), "different inputs_fp must miss");
    }

    #[test]
    fn cache_persists_to_disk() {
        let dir = temp_dir("persist");
        let p = dir.join("build.json");
        let obj = dir.join("out.o");
        fs::write(&obj, b"\x7fELF").unwrap();
        {
            let mut c = make_cache(&p);
            c.insert(10, 20, 30, "arm", &obj).unwrap();
        }
        let c2 = make_cache(&p);
        assert!(c2.lookup(10, 20, 30).is_some());
    }

    #[test]
    fn abi_hash_isolation() {
        let dir = temp_dir("abi_iso");
        let p = dir.join("build.json");
        let mut c = make_cache(&p);
        let oa = dir.join("a.o");
        let ob = dir.join("b.o");
        fs::write(&oa, b"a").unwrap();
        fs::write(&ob, b"b").unwrap();
        c.insert(1, 2, 100, "t", &oa).unwrap();
        c.insert(1, 2, 200, "t", &ob).unwrap();
        assert_eq!(c.lookup(1, 2, 100).unwrap().object_path, oa);
        assert_eq!(c.lookup(1, 2, 200).unwrap().object_path, ob);
    }

    #[test]
    fn version_mismatch_treated_as_cold() {
        let dir = temp_dir("ver_mismatch");
        let p = dir.join("build.json");
        let mut c = make_cache(&p);
        let obj = dir.join("out.o");
        fs::write(&obj, b"\x7fELF").unwrap();
        c.insert(1, 2, 3, "t", &obj).unwrap();
        // Bump version in file.
        let raw = fs::read_to_string(&p).unwrap();
        let bumped = raw.replace("\"version\": 2", "\"version\": 3");
        fs::write(&p, &bumped).unwrap();
        let c2 = make_cache(&p);
        assert!(c2.artifacts.is_empty(), "wrong version = cold");
    }

    #[test]
    fn fnv1a_is_deterministic() {
        assert_eq!(fnv1a_u64(b"hello"), fnv1a_u64(b"hello"));
    }

    #[test]
    fn compiler_fp_stable_within_session() {
        let a = compiler_fingerprint();
        let b = compiler_fingerprint();
        assert_eq!(a, b, "compiler_fp must be stable within a process");
    }

    #[test]
    fn inputs_fp_changes_on_different_own_hash() {
        let a = inputs_fingerprint(1, "t", &[]);
        let b = inputs_fingerprint(2, "t", &[]);
        assert_ne!(a, b);
    }

    #[test]
    fn inputs_fp_changes_on_different_triple() {
        let a = inputs_fingerprint(1, "x86", &[]);
        let b = inputs_fingerprint(1, "arm", &[]);
        assert_ne!(a, b);
    }

    #[test]
    fn inputs_fp_changes_on_different_dep_hashes() {
        let a = inputs_fingerprint(1, "t", &[10, 20]);
        let b = inputs_fingerprint(1, "t", &[10]);
        assert_ne!(a, b);
    }

    #[test]
    fn inputs_fp_is_deterministic() {
        let a = inputs_fingerprint(1, "t", &[20, 10]);
        let b = inputs_fingerprint(1, "t", &[10, 20]);
        assert_eq!(a, b, "inputs_fp must be order-independent");
    }

    #[test]
    fn old_format_v1_treated_as_cold() {
        let dir = temp_dir("old_v1");
        let p = dir.join("build.json");
        // Write v1 format (no compiler_fp/inputs_fp fields).
        let v1 = r#"{"version":1,"artifacts":{}}"#;
        fs::write(&p, v1).unwrap();
        let c = make_cache(&p);
        assert!(c.artifacts.is_empty(), "v1 cache must be cold");
    }
}
