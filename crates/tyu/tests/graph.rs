//! Tests for the module dependency graph resolver.

use std::fs;
use std::path::PathBuf;

use tyu::graph;
use tyu::test_helpers::temp_dir;

fn write_mod(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(format!("{}.mod", name));
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn single_module_no_imports() {
    let dir = temp_dir("single");
    let main_mod = write_mod(&dir, "main", "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n");
    let g = graph::resolve_graph(&main_mod, &[], None).unwrap();
    assert_eq!(g.len(), 1);
    assert!(!g[0].is_lib);
}

#[test]
fn diamond_import_resolved_once() {
    let dir = temp_dir("diamond");
    write_mod(&dir, "Leaf", "module Leaf;\n: leaf-fn ( -- i64 ) 42 ;\nexport { leaf-fn };\nend;\n");
    write_mod(&dir, "Left", "module Left;\nimport Leaf { leaf-fn };\n: left-fn ( -- i64 ) leaf-fn ;\nexport { left-fn };\nend;\n");
    write_mod(&dir, "Right", "module Right;\nimport Leaf { leaf-fn };\n: right-fn ( -- i64 ) leaf-fn ;\nexport { right-fn };\nend;\n");
    let main_path = write_mod(&dir, "main", "module Main;\nimport Left { left-fn };\nimport Right { right-fn };\n: main ( -- i64 ) left-fn right-fn + ;\nexport { main };\nend;\n");
    let g = graph::resolve_graph(&main_path, &[], None).unwrap();
    assert_eq!(g.len(), 4);
    assert_eq!(g[0].name, "Leaf");
    assert_eq!(g[g.len() - 1].name, "Main");
    assert!(g[0].is_lib);
    assert!(!g[g.len() - 1].is_lib);
}

#[test]
fn chain_import_order() {
    let dir = temp_dir("chain");
    write_mod(&dir, "C", "module C;\n: c-fn ( -- i64 ) 1 ;\nexport { c-fn };\nend;\n");
    write_mod(&dir, "B", "module B;\nimport C { c-fn };\n: b-fn ( -- i64 ) c-fn ;\nexport { b-fn };\nend;\n");
    let a = write_mod(&dir, "A", "module A;\nimport B { b-fn };\n: a-fn ( -- i64 ) b-fn ;\nexport { a-fn };\nend;\n");
    let g = graph::resolve_graph(&a, &[], None).unwrap();
    assert_eq!(g.len(), 3);
    assert_eq!(g[0].name, "C");
    assert_eq!(g[1].name, "B");
    assert_eq!(g[2].name, "A");
}

#[test]
fn no_double_count_on_same_import() {
    let dir = temp_dir("lib_once");
    write_mod(&dir, "Lib", "module Lib;\n: helper ( -- i64 ) 7 ;\nexport { helper };\nend;\n");
    let main_path = write_mod(&dir, "main", "module Main;\nimport Lib { helper };\n: main ( -- i64 ) helper ;\nexport { main };\nend;\n");
    let g = graph::resolve_graph(&main_path, &[], None).unwrap();
    assert_eq!(g.len(), 2);
}

#[test]
fn resolve_via_include_dirs() {
    let dir = temp_dir("inc_dirs");
    let lib_dir = dir.join("libs");
    fs::create_dir_all(&lib_dir).unwrap();
    let main_dir = dir.join("src");
    fs::create_dir_all(&main_dir).unwrap();
    write_mod(&lib_dir, "Mylib", "module Mylib;\n: lib-fn ( -- i64 ) 99 ;\nexport { lib-fn };\nend;\n");
    let main_path = write_mod(&main_dir, "main", "module Main;\nimport Mylib { lib-fn };\n: main ( -- i64 ) lib-fn ;\nexport { main };\nend;\n");
    let g = graph::resolve_graph(&main_path, &[lib_dir.clone()], None).unwrap();
    assert_eq!(g.len(), 2);
    assert!(g.iter().any(|n| n.name == "Mylib"));
}

#[test]
fn platform_import_ignored_if_not_found() {
    let dir = temp_dir("platform_ignored");
    let main_path = write_mod(&dir, "main", "module Main;\nimport platform/testio { testio.write-byte };\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n");
    let g = graph::resolve_graph(&main_path, &[], None).unwrap();
    assert_eq!(g.len(), 1);
}
