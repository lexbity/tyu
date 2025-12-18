use std::{
    path::PathBuf,
    process::Command,
    sync::Once,
};

static BUILD_ONCE: Once = Once::new();

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_tools() {
    BUILD_ONCE.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .current_dir(workspace_root())
            .args(["build", "-q"])
            .status()
            .expect("cargo build");
        assert!(status.success());
    });
}

fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_lang_tests")
        .join(format!("{}_{}", name, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[test]
fn langc_help() {
    build_tools();
    let out = Command::new(exe("langc")).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("USAGE:"));
    assert!(stdout.contains("langc"));
}

#[test]
fn lang_assemble_help() {
    build_tools();
    let out = Command::new(exe("lang-assemble"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("USAGE:"));
    assert!(stdout.contains("lang-assemble"));
}

#[test]
fn hosted_read_file_roundtrip() {
    let dir = fresh_dir("hosted_read_file_roundtrip");
    let path = dir.join("hosted_read_file_roundtrip.txt");
    std::fs::write(&path, b"abc123").unwrap();

    let buf = hosted::fs::read_file(path.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(buf.as_slice(), b"abc123");
}

#[test]
fn langc_emit_ast_simple_module() {
    build_tools();
    let dir = fresh_dir("langc_emit_ast_simple_module");
    let path = dir.join("demo.mod");
    let core_def = dir.join("Core.def");

    let core_src = b"module Core;\nexport { add3 add };\n: add3 ( a b c -- sum ) ;\n: add ( a b -- sum ) ;\nend;\n";
    std::fs::write(&core_def, core_src).unwrap();

    let src = b"module Demo;\nimport Core { add3, add };\nexport { add3 };\n: add3 ( a b c -- sum )\n  + +\n;\nstruct Point\n  x : i32\n  y : i32\nend;\nend;\n";
    std::fs::write(&path, src).unwrap();

    let out = Command::new(exe("langc"))
        .args(["--emit=ast", path.to_string_lossy().as_ref()])
        .output()
        .unwrap();
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = "\
module Demo
  import Core {add3 add}
  export {add3}
  word add3
    sig ( a b c -- sum )
    body + +
  decl struct Point
";
    assert_eq!(stdout, expected);
}

#[test]
fn iface_conformance_ok() {
    build_tools();
    let dir = fresh_dir("iface_conformance_ok");

    std::fs::write(
        dir.join("Core.def"),
        b"module Core;\nexport { add };\n: add ( a b -- sum ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Core.mod"),
        b"module Core;\nexport { add };\n: add ( a b -- sum )\n  +\n;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { add };\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ast", "Main.mod"])
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn iface_signature_mismatch_fails() {
    build_tools();
    let dir = fresh_dir("iface_signature_mismatch_fails");

    std::fs::write(
        dir.join("Core.def"),
        b"module Core;\nexport { add };\n: add ( a b -- sum ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Core.mod"),
        b"module Core;\nexport { add };\n: add ( a b c -- sum )\n  + +\n;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { add };\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ast", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E2218]"));
}

#[test]
fn import_missing_symbol_fails() {
    build_tools();
    let dir = fresh_dir("import_missing_symbol_fails");

    std::fs::write(
        dir.join("Core.def"),
        b"module Core;\nexport { add };\n: add ( a b -- sum ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { missing };\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ast", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E2203]"));
}

#[test]
fn langc_emit_ir_typechecks_if_while() {
    build_tools();
    let dir = fresh_dir("langc_emit_ir_typechecks_if_while");

    // Provide a Core.def so import resolution passes (even though we only use builtins in this test).
    std::fs::write(dir.join("Core.def"), b"module Core;\nexport { };\nend;\n").unwrap();

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { };\n: pick ( i64 i64 bool -- i64 )\n  [ drop ] [ swap drop ] if\n;\n: countdown ( i64 -- i64 )\n  [ dup 0 > ] [ 1 - ] while\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("module Main"));
    assert!(stdout.contains("word pick"));
    assert!(stdout.contains("if | stack: i64"));
    assert!(stdout.contains("word countdown"));
    assert!(stdout.contains("while | stack: i64"));
}

#[test]
fn langc_emit_ir_rejects_type_mismatch() {
    build_tools();
    let dir = fresh_dir("langc_emit_ir_rejects_type_mismatch");
    std::fs::write(dir.join("Core.def"), b"module Core;\nexport { };\nend;\n").unwrap();

    // `pick` expects `bool` for `if` condition, but pushes an `i64`.
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { };\n: pick ( i64 i64 bool -- i64 )\n  0 [ drop ] [ swap drop ] if\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3243]") || stderr.contains("error[E3242]"));
}

#[test]
fn milestone4_contracts_and_subtypes_in_ir() {
    build_tools();
    let dir = fresh_dir("milestone4_contracts_and_subtypes_in_ir");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nsubtype Percent = i64 range 0..100;\n: clamp ( i64 -- Percent )\n  as Percent\n;\n: pwm_set ( Percent -- )\n  requires [ dup dup 0 >= swap 100 <= and ]\n  drop\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("check_subtype Percent"));
    assert!(stdout.contains("trap_if_false SUBTYPE_FAIL"));
    assert!(stdout.contains("check_param Percent"));
    assert!(stdout.contains("requires"));
    assert!(stdout.contains("trap_if_false CONTRACT_FAIL"));
}

#[test]
fn milestone4_checks_flag_controls_insertion() {
    build_tools();
    let dir = fresh_dir("milestone4_checks_flag_controls_insertion");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nsubtype Percent = i64 range 0..100;\n: clamp ( i64 -- Percent ) as Percent ;\n: pwm_set ( Percent -- ) requires [ dup dup 0 >= swap 100 <= and ] drop ;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--checks=contracts", "Main.mod"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("trap_if_false CONTRACT_FAIL"));
    assert!(!stdout.contains("SUBTYPE_FAIL"));

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--checks=off", "Main.mod"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("CONTRACT_FAIL"));
    assert!(!stdout.contains("SUBTYPE_FAIL"));
}
