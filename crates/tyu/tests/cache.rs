//! Tests for the build cache.

use std::fs;
use std::path::PathBuf;

use tyu::cache::{content_hash, BuildCache};

/// Helper to create a temp dir for a test.
fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_cache_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn content_hash_is_deterministic() {
    let dir = temp_dir("hash_det");
    let path = dir.join("test.txt");
    fs::write(&path, b"hello world").unwrap();

    let h1 = content_hash(&path).unwrap();
    let h2 = content_hash(&path).unwrap();
    assert_eq!(h1, h2, "same file content must produce same hash");
}

#[test]
fn different_content_different_hash() {
    let dir = temp_dir("hash_diff");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    fs::write(&a, b"hello").unwrap();
    fs::write(&b, b"world").unwrap();

    let ha = content_hash(&a).unwrap();
    let hb = content_hash(&b).unwrap();
    assert_ne!(ha, hb, "different content must produce different hash");
}

#[test]
fn cache_lookup_miss_on_unknown_key() {
    let dir = temp_dir("lookup_miss");
    let cache_path = dir.join("build.json");
    let mut cache = BuildCache::load(&cache_path);

    let src = dir.join("src.mod");
    fs::write(&src, b"module M;\nend;\n").unwrap();

    let result = cache.lookup(&src, "x86_64-unknown-none", 42).unwrap();
    assert!(result.is_none(), "cache miss should return None");
}

#[test]
fn cache_insert_then_lookup_hit() {
    let dir = temp_dir("insert_lookup");
    let cache_path = dir.join("build.json");
    let mut cache = BuildCache::load(&cache_path);

    let src = dir.join("src.mod");
    fs::write(&src, b"module M;\n: f 0 ;\nend;\n").unwrap();

    let obj = dir.join("out.o");
    fs::write(&obj, b"\x7fELF").unwrap();

    cache.insert(&src, "x86_64-unknown-none", 42, &obj).unwrap();

    let result = cache.lookup(&src, "x86_64-unknown-none", 42).unwrap();
    assert!(result.is_some(), "cache hit after insert");
    assert_eq!(result.unwrap().object_path, obj);
}

#[test]
fn cache_lookup_miss_on_changed_source() {
    let dir = temp_dir("miss_changed");
    let cache_path = dir.join("build.json");
    let mut cache = BuildCache::load(&cache_path);

    let src = dir.join("src.mod");
    fs::write(&src, b"module M;\n: f 0 ;\nend;\n").unwrap();
    let obj = dir.join("out.o");
    fs::write(&obj, b"\x7fELF").unwrap();

    cache.insert(&src, "x86_64-unknown-none", 42, &obj).unwrap();

    // Change the source content.
    fs::write(&src, b"module M;\n: g 1 ;\nend;\n").unwrap();

    let result = cache.lookup(&src, "x86_64-unknown-none", 42).unwrap();
    assert!(result.is_none(), "cache miss after source change");
}

#[test]
fn cache_persists_to_disk() {
    let dir = temp_dir("persist");
    let cache_path = dir.join("build.json");

    let src = dir.join("src.mod");
    fs::write(&src, b"module M;\nend;\n").unwrap();
    let obj = dir.join("out.o");
    fs::write(&obj, b"\x7fELF").unwrap();

    // Insert via one cache instance.
    {
        let mut cache = BuildCache::load(&cache_path);
        cache.insert(&src, "armv7m-unknown-none", 99, &obj).unwrap();
    }

    // Load a fresh instance — should persist.
    let cache2 = BuildCache::load(&cache_path);
    let result = cache2.lookup(&src, "armv7m-unknown-none", 99).unwrap();
    assert!(result.is_some(), "cache must persist to disk");
    assert_eq!(result.unwrap().object_path, obj);
}

#[test]
fn cache_abi_hash_isolation() {
    // Different abi_hash values should produce cache misses for same source.
    let dir = temp_dir("abi_isolation");
    let cache_path = dir.join("build.json");
    let mut cache = BuildCache::load(&cache_path);

    let src = dir.join("src.mod");
    fs::write(&src, b"module M;\nend;\n").unwrap();
    let obj_a = dir.join("out_a.o");
    let obj_b = dir.join("out_b.o");
    fs::write(&obj_a, b"\x7fELF").unwrap();
    fs::write(&obj_b, b"\x7fELF").unwrap();

    cache.insert(&src, "x86_64-unknown-none", 100, &obj_a).unwrap();
    cache.insert(&src, "x86_64-unknown-none", 200, &obj_b).unwrap();

    let r1 = cache.lookup(&src, "x86_64-unknown-none", 100).unwrap();
    assert_eq!(r1.unwrap().object_path, obj_a);

    let r2 = cache.lookup(&src, "x86_64-unknown-none", 200).unwrap();
    assert_eq!(r2.unwrap().object_path, obj_b);
}

#[test]
fn cache_target_isolation() {
    // Different targets should produce cache misses for same source.
    let dir = temp_dir("target_isolation");
    let cache_path = dir.join("build.json");
    let mut cache = BuildCache::load(&cache_path);

    let src = dir.join("src.mod");
    fs::write(&src, b"module M;\nend;\n").unwrap();
    let obj_x = dir.join("out_x.o");
    let obj_a = dir.join("out_a.o");
    fs::write(&obj_x, b"\x7fELF").unwrap();
    fs::write(&obj_a, b"\x7fELF").unwrap();

    cache.insert(&src, "x86_64-unknown-none", 0, &obj_x).unwrap();
    cache.insert(&src, "armv7m-unknown-none", 0, &obj_a).unwrap();

    let rx = cache.lookup(&src, "x86_64-unknown-none", 0).unwrap();
    assert_eq!(rx.unwrap().object_path, obj_x);

    let ra = cache.lookup(&src, "armv7m-unknown-none", 0).unwrap();
    assert_eq!(ra.unwrap().object_path, obj_a);
}
