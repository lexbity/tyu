//! Tests for the build cache (new key: compiler_fp + inputs_fp + abi_hash).

use std::fs;
use std::path::PathBuf;

use tyu::cache::{compiler_fingerprint, inputs_fingerprint, BuildCache};

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_cache_tests").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn lookup_miss_on_unknown_key() {
    let dir = temp_dir("lookup_miss");
    let cache = BuildCache::load(&dir.join("build.json"));
    assert!(
        cache.lookup(0, 0, 0).is_none(),
        "cache miss should return None"
    );
}

#[test]
fn insert_then_lookup_hit() {
    let dir = temp_dir("insert_lookup");
    let p = dir.join("build.json");
    let mut cache = BuildCache::load(&p);
    let obj = dir.join("out.o");
    fs::write(&obj, b"\x7fELF").unwrap();

    cache.insert(1, 2, 42, "x86_64-unknown-none", &obj).unwrap();

    let result = cache.lookup(1, 2, 42);
    assert!(result.is_some(), "cache hit after insert");
    assert_eq!(result.unwrap().object_path, obj);
}

#[test]
fn lookup_miss_on_different_compiler_fp() {
    let dir = temp_dir("compiler_miss");
    let p = dir.join("build.json");
    let mut cache = BuildCache::load(&p);
    let obj = dir.join("out.o");
    fs::write(&obj, b"\x7fELF").unwrap();

    cache.insert(1, 2, 42, "t", &obj).unwrap();
    assert!(
        cache.lookup(99, 2, 42).is_none(),
        "different compiler_fp must miss"
    );
}

#[test]
fn lookup_miss_on_different_inputs_fp() {
    let dir = temp_dir("inputs_miss");
    let p = dir.join("build.json");
    let mut cache = BuildCache::load(&p);
    let obj = dir.join("out.o");
    fs::write(&obj, b"\x7fELF").unwrap();

    cache.insert(1, 2, 42, "t", &obj).unwrap();
    assert!(
        cache.lookup(1, 99, 42).is_none(),
        "different inputs_fp must miss"
    );
}

#[test]
fn cache_persists_to_disk() {
    let dir = temp_dir("persist");
    let p = dir.join("build.json");
    let obj = dir.join("out.o");
    fs::write(&obj, b"\x7fELF").unwrap();

    {
        let mut cache = BuildCache::load(&p);
        cache
            .insert(10, 20, 99, "armv7m-unknown-none", &obj)
            .unwrap();
    }

    let cache2 = BuildCache::load(&p);
    let result = cache2.lookup(10, 20, 99);
    assert!(result.is_some(), "cache must persist to disk");
    assert_eq!(result.unwrap().object_path, obj);
}

#[test]
fn abi_hash_isolation() {
    let dir = temp_dir("abi_iso");
    let p = dir.join("build.json");
    let mut cache = BuildCache::load(&p);
    let oa = dir.join("out_a.o");
    let ob = dir.join("out_b.o");
    fs::write(&oa, b"\x7fELF").unwrap();
    fs::write(&ob, b"\x7fELF").unwrap();

    cache.insert(1, 2, 100, "x86_64-unknown-none", &oa).unwrap();
    cache.insert(1, 2, 200, "x86_64-unknown-none", &ob).unwrap();

    let r1 = cache.lookup(1, 2, 100).unwrap();
    assert_eq!(r1.object_path, oa);

    let r2 = cache.lookup(1, 2, 200).unwrap();
    assert_eq!(r2.object_path, ob);
}

#[test]
fn compiler_fp_stable_within_session() {
    let a = compiler_fingerprint();
    let b = compiler_fingerprint();
    assert_eq!(a, b, "compiler_fp must be stable within a process");
}

#[test]
fn inputs_fp_deterministic() {
    let a = inputs_fingerprint(1, "test", &[10, 20]);
    let b = inputs_fingerprint(1, "test", &[20, 10]);
    assert_eq!(a, b, "inputs_fp must be order-independent");
}

#[test]
fn inputs_fp_differs_on_different_hash() {
    let a = inputs_fingerprint(1, "t", &[]);
    let b = inputs_fingerprint(2, "t", &[]);
    assert_ne!(a, b);
}

#[test]
fn inputs_fp_differs_on_different_triple() {
    let a = inputs_fingerprint(1, "x86", &[]);
    let b = inputs_fingerprint(1, "arm", &[]);
    assert_ne!(a, b);
}

#[test]
fn inputs_fp_differs_on_different_deps() {
    let a = inputs_fingerprint(1, "t", &[10]);
    let b = inputs_fingerprint(1, "t", &[10, 20]);
    assert_ne!(a, b);
}
