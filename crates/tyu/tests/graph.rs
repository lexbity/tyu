//! Tests for the module dependency graph resolver.

use std::fs;
use std::path::PathBuf;

/// Path helper: get the workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Create a temporary .mod file for testing.
fn write_mod(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(format!("{}.mod", name));
    fs::write(&path, content).unwrap();
    path
}

/// Helper to create a temp dir for a test.
fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_graph_tests")
        .join(format!("{}_{}", label, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn single_module_no_imports() {
    let dir = temp_dir("single");
    let main_mod = write_mod(&dir, "main", "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n");

    let graph = tyu::graph::resolve_graph(&main_mod, &[], None).unwrap();
    assert_eq!(graph.len(), 1, "single module should produce one node");
    assert!(!graph[0].is_lib, "root module should not be lib");
}

#[test]
fn diamond_import_resolved_once() {
    // diamond.mod imports leaf.mod and left.mod and right.mod
    // left.mod imports leaf.mod
    // right.mod imports leaf.mod
    // leaf.mod should appear only once
    let dir = temp_dir("diamond");

    let leaf = write_mod(&dir, "Leaf",
        "module Leaf;\n: leaf-fn ( -- i64 ) 42 ;\nexport { leaf-fn };\nend;\n");
    let left = write_mod(&dir, "Left",
        "module Left;\nimport Leaf { leaf-fn };\n: left-fn ( -- i64 ) leaf-fn ;\nexport { left-fn };\nend;\n");
    let right = write_mod(&dir, "Right",
        "module Right;\nimport Leaf { leaf-fn };\n: right-fn ( -- i64 ) leaf-fn ;\nexport { right-fn };\nend;\n");
    let main_path = write_mod(&dir, "main",
        "module Main;\nimport Left { left-fn };\nimport Right { right-fn };\n: main ( -- i64 ) left-fn right-fn + ;\nexport { main };\nend;\n");

    let graph = tyu::graph::resolve_graph(&main_path, &[], None).unwrap();

    // Should have 4 modules (Main, Left, Right, Leaf) — Leaf should appear once.
    assert_eq!(graph.len(), 4, "diamond should produce exactly 4 nodes");

    // Leaf should be first (it has no deps), Main should be last.
    assert_eq!(graph[0].name, "Leaf", "leaf should be first (no deps)");
    assert_eq!(graph[graph.len() - 1].name, "Main", "main should be last");
    assert!(graph[0].is_lib, "leaf should be lib");
    assert!(!graph[graph.len() - 1].is_lib, "main should not be lib");
}

#[test]
fn chain_import_order() {
    // A imports B, B imports C → order: C, B, A
    let dir = temp_dir("chain");

    let c = write_mod(&dir, "C",
        "module C;\n: c-fn ( -- i64 ) 1 ;\nexport { c-fn };\nend;\n");
    let b = write_mod(&dir, "B",
        "module B;\nimport C { c-fn };\n: b-fn ( -- i64 ) c-fn ;\nexport { b-fn };\nend;\n");
    let a = write_mod(&dir, "A",
        "module A;\nimport B { b-fn };\n: a-fn ( -- i64 ) b-fn ;\nexport { a-fn };\nend;\n");

    let graph = tyu::graph::resolve_graph(&a, &[], None).unwrap();
    assert_eq!(graph.len(), 3);
    assert_eq!(graph[0].name, "C", "C should be first (no deps)");
    assert_eq!(graph[1].name, "B", "B should be second");
    assert_eq!(graph[2].name, "A", "A should be last (root)");
}

#[test]
fn no_double_count_on_same_import() {
    // Main imports Lib — Lib should not be doubled.
    let dir = temp_dir("lib_once");

    let lib = write_mod(&dir, "Lib",
        "module Lib;\n: helper ( -- i64 ) 7 ;\nexport { helper };\nend;\n");
    let main_path = write_mod(&dir, "main",
        "module Main;\nimport Lib { helper };\n: main ( -- i64 ) helper ;\nexport { main };\nend;\n");

    let graph = tyu::graph::resolve_graph(&main_path, &[], None).unwrap();
    assert_eq!(graph.len(), 2, "main + lib = 2 nodes");
}

#[test]
fn resolve_via_include_dirs() {
    let dir = temp_dir("inc_dirs");
    let lib_dir = dir.join("libs");
    fs::create_dir_all(&lib_dir).unwrap();
    let main_dir = dir.join("src");
    fs::create_dir_all(&main_dir).unwrap();

    // Put the library in libs/ and the main in src/
    write_mod(&lib_dir, "Mylib",
        "module Mylib;\n: lib-fn ( -- i64 ) 99 ;\nexport { lib-fn };\nend;\n");
    let main_path = write_mod(&main_dir, "main",
        "module Main;\nimport Mylib { lib-fn };\n: main ( -- i64 ) lib-fn ;\nexport { main };\nend;\n");

    let graph = tyu::graph::resolve_graph(&main_path, &[lib_dir.clone()], None)
        .expect("should resolve via include dirs");
    assert_eq!(graph.len(), 2, "main + mylib = 2 nodes");
    assert!(graph.iter().any(|n| n.name == "Mylib"), "should find Mylib");
}

#[test]
fn platform_import_ignored_if_not_found() {
    // platform/testio is in the sysroot but if no sysroot is given, it should
    // be silently ignored (treated as externally provided).
    let dir = temp_dir("platform_ignored");

    let main_path = write_mod(&dir, "main",
        "module Main;\nimport platform/testio { testio.write-byte };\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n");

    let graph = tyu::graph::resolve_graph(&main_path, &[], None)
        .expect("should resolve without sysroot — platform import is external");
    assert_eq!(graph.len(), 1, "only main — platform/testio is treated as external");
}
