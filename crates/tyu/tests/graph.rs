//! Tests for the module dependency graph resolver.
//!
//! Note: ModuleAst is ~156KB on the stack (FixedVec inline storage).
//! Each test runs on a thread with 8MB stack.

use std::fs;
use std::path::PathBuf;

use tyu::graph;
use tyu::test_helpers::temp_dir;

/// Run `f` on a thread with 8MB stack (ModuleAst ~156KB needs it).
fn with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn write_mod(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(format!("{}.mod", name));
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn single_module_no_imports() {
    with_big_stack(|| {
        let dir = temp_dir("single");
        let main_mod = write_mod(&dir, "main", "module Main;\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n");
        let g = graph::resolve_graph(&main_mod, &[], None).unwrap();
        assert_eq!(g.len(), 1);
        assert!(!g[0].is_lib);
    })
}

#[test]
fn diamond_import_resolved_once() {
    with_big_stack(|| {
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
    })
}

#[test]
fn chain_import_order() {
    with_big_stack(|| {
        let dir = temp_dir("chain");
        write_mod(&dir, "C", "module C;\n: c-fn ( -- i64 ) 1 ;\nexport { c-fn };\nend;\n");
        write_mod(&dir, "B", "module B;\nimport C { c-fn };\n: b-fn ( -- i64 ) c-fn ;\nexport { b-fn };\nend;\n");
        let a = write_mod(&dir, "A", "module A;\nimport B { b-fn };\n: a-fn ( -- i64 ) b-fn ;\nexport { a-fn };\nend;\n");
        let g = graph::resolve_graph(&a, &[], None).unwrap();
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].name, "C");
        assert_eq!(g[1].name, "B");
        assert_eq!(g[2].name, "A");
    })
}

#[test]
fn no_double_count_on_same_import() {
    with_big_stack(|| {
        let dir = temp_dir("lib_once");
        write_mod(&dir, "Lib", "module Lib;\n: helper ( -- i64 ) 7 ;\nexport { helper };\nend;\n");
        let main_path = write_mod(&dir, "main", "module Main;\nimport Lib { helper };\n: main ( -- i64 ) helper ;\nexport { main };\nend;\n");
        let g = graph::resolve_graph(&main_path, &[], None).unwrap();
        assert_eq!(g.len(), 2);
    })
}

#[test]
fn resolve_via_include_dirs() {
    with_big_stack(|| {
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
    })
}

#[test]
fn platform_import_ignored_if_not_found() {
    with_big_stack(|| {
        let dir = temp_dir("platform_ignored");
        let main_path = write_mod(&dir, "main", "module Main;\nimport platform/testio { testio.write-byte };\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n");
        let g = graph::resolve_graph(&main_path, &[], None).unwrap();
        assert_eq!(g.len(), 1);
    })
}

#[test]
fn cycle_detected_errors() {
    with_big_stack(|| {
        let dir = temp_dir("g1_cycle");
        write_mod(&dir, "B", "module B;\nimport A { a-fn };\n: b-fn ( -- i64 ) a-fn ;\nexport { b-fn };\nend;\n");
        let main_path = write_mod(&dir, "A", "module A;\nimport B { b-fn };\n: a-fn ( -- i64 ) b-fn ;\nexport { a-fn };\nend;\n");
        let result = graph::resolve_graph(&main_path, &[], None);
        assert!(result.is_err(), "G-1: A→B→A cycle must be detected");
        let err = result.unwrap_err();
        assert!(err.contains("circular"), "G-1: error must mention 'circular', got: {}", err);
    })
}

#[test]
fn missing_user_module_errors() {
    with_big_stack(|| {
        let dir = temp_dir("g2_missing");
        let main_path = write_mod(&dir, "main", "module Main;\nimport NonExistent { some-fn };\n: main ( -- i64 ) 0 ;\nexport { main };\nend;\n");
        let result = graph::resolve_graph(&main_path, &[], None);
        assert!(result.is_err(), "G-2: missing user module must error");
        let err = result.unwrap_err();
        assert!(err.contains("NonExistent"), "G-2: error must mention module name, got: {}", err);
    })
}

#[test]
fn include_dir_precedence() {
    with_big_stack(|| {
        let dir = temp_dir("g3_prec");
        let lib_dir = dir.join("libs");
        fs::create_dir_all(&lib_dir).unwrap();
        let main_dir = dir.join("src");
        fs::create_dir_all(&main_dir).unwrap();
        write_mod(&main_dir, "Util", "module Util;\n: helper ( -- i64 ) 1 ;\nexport { helper };\nend;\n");
        write_mod(&lib_dir, "Util", "module Util;\n: helper ( -- i64 ) 99 ;\nexport { helper };\nend;\n");
        let main_path = write_mod(&main_dir, "main", "module Main;\nimport Util { helper };\n: main ( -- i64 ) helper ;\nexport { main };\nend;\n");
        let g = graph::resolve_graph(&main_path, &[lib_dir.clone()], None).unwrap();
        assert_eq!(g.len(), 2, "G-3: must resolve Main + Util");
        let util_node = g.iter().find(|n| n.name == "Util").unwrap();
        assert_eq!(util_node.path.parent(), Some(main_dir.as_path()),
            "G-3: entry-dir Util must win over include-dir Util, got path: {}",
            util_node.path.display());
    })
}
