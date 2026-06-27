use std::{path::PathBuf, process::Command, sync::Once};

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
        // Capture stdio so the nested cargo never flips the shared test
        // terminal to O_NONBLOCK; that flag leaks back to the outer
        // `cargo test` harness, whose writes then panic with EAGAIN.
        let out = Command::new(env!("CARGO"))
            .current_dir(workspace_root())
            .args(["build", "-q"])
            .output()
            .expect("cargo build");
        assert!(
            out.status.success(),
            "cargo build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    });
}

fn exe(name: &str) -> PathBuf {
    workspace_root().join("target").join("debug").join(name)
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_lang_tests").join(format!(
        "{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn repo_sysroot() -> PathBuf {
    workspace_root().join("sysroot")
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
fn langc_unknown_target_triple_fails() {
    build_tools();
    let dir = fresh_dir("langc_unknown_target_triple_fails");
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 ) 0 ;\nend;\n",
    )
    .unwrap();
    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--target=not-a-real-target", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("E1019"),
        "expected E1019 in stderr: {stderr}"
    );
}

#[test]
fn langc_obj_without_target_fails() {
    build_tools();
    let dir = fresh_dir("langc_obj_without_target_fails");
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 ) 0 ;\nend;\n",
    )
    .unwrap();
    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=obj", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("E1020"),
        "expected E1020 in stderr: {stderr}"
    );
}

#[test]
fn lang_assemble_unknown_target_triple_fails() {
    build_tools();
    let dir = fresh_dir("lang_assemble_unknown_target_triple_fails");
    std::fs::write(dir.join("foo.asm"), b"; dummy\n").unwrap();
    let out = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--target=not-a-real-target", "foo.asm"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("E2004"),
        "expected E2004 in stderr: {stderr}"
    );
}

#[test]
fn sysroot_two_level_layout_resolves_target_platform_modules() {
    // Regression test for the two-level sysroot restructure:
    //   sysroot/                                    ← root (target-agnostic)
    //   sysroot/x86_64-unknown-linux-gnu/platform/  ← target-specific modules
    //
    // `import platform/myplatform` must resolve through the target subdir.
    build_tools();
    let dir = fresh_dir("sysroot_two_level_layout");

    let platform_dir = dir
        .join("sysroot")
        .join("x86_64-unknown-linux-gnu")
        .join("platform");
    std::fs::create_dir_all(&platform_dir).unwrap();

    // Minimal interface: one word with a known signature.
    std::fs::write(
        platform_dir.join("myplatform.def"),
        b"module platform/myplatform;\nexport { myfn };\n: myfn ( -- i64 ) ;\nend;\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport platform/myplatform { myfn };\n: main ( -- i64 ) myfn ;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=ir",
            &format!("--sysroot={}", dir.join("sysroot").to_string_lossy()),
            "Main.mod",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
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
fn milestone1_emit_ast_owned_iso_decls() {
    build_tools();
    let dir = fresh_dir("milestone1_emit_ast_owned_iso_decls");
    let path = dir.join("Demo.mod");

    let src = b"module Demo;\n\
owned Buffer;\n\
iso Message;\n\
: main ( -- i64 )\n\
  0\n\
;\n\
end;\n";
    std::fs::write(&path, src).unwrap();

    let out = Command::new(exe("langc"))
        .args(["--emit=ast", path.to_string_lossy().as_ref()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = "\
module Demo
  decl owned Buffer
  decl iso Message
  word main
    sig ( -- i64 )
    body 0
";
    assert_eq!(stdout, expected);
}

#[test]
fn milestone2_import_missing_def_has_stable_diag() {
    build_tools();
    let dir = fresh_dir("milestone2_import_missing_def_has_stable_diag");
    let path = dir.join("Main.mod");

    std::fs::write(
        &path,
        b"module Main;\nimport Missing { x };\n: main ( -- i64 ) 0 ;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ast", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E2201]"));
    assert!(stderr.contains("import interface file (.def) not found"));
}

#[test]
fn milestone2_import_symbol_not_exported_has_stable_diag() {
    build_tools();
    let dir = fresh_dir("milestone2_import_symbol_not_exported_has_stable_diag");

    std::fs::write(
        dir.join("Core.def"),
        b"module Core;\nexport { a };\n: a ( -- i64 ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { b };\n: main ( -- i64 ) 0 ;\nend;\n",
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
    assert!(stderr.contains("imported symbol not exported by interface"));
}

#[test]
fn milestone2_iface_sig_mismatch_has_stable_diag() {
    build_tools();
    let dir = fresh_dir("milestone2_iface_sig_mismatch_has_stable_diag");

    std::fs::write(
        dir.join("Core.def"),
        b"module Core;\nexport { add };\n: add ( a b -- sum ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Core.mod"),
        b"module Core;\nexport { add };\n: add ( a b c -- sum ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { add };\n: main ( -- i64 ) 0 ;\nend;\n",
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
    assert!(stderr.contains("interface/implementation word signature mismatch"));
}

#[test]
fn milestone2_sysroot_env_resolves_platform_imports() {
    build_tools();
    let dir = fresh_dir("milestone2_sysroot_env_resolves_platform_imports");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { platform.io.log };\n\
: main ( -- i64 )\n\
  \"hi\" platform.io.log\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .env("LANG_SYSROOT", repo_sysroot())
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn milestone2_sysroot_flag_overrides_env() {
    build_tools();
    let dir = fresh_dir("milestone2_sysroot_flag_overrides_env");

    // Provide a "bad" sysroot via env (doesn't contain platform/linux.def).
    let bad_sysroot = dir.join("bad_sysroot");
    std::fs::create_dir_all(&bad_sysroot).unwrap();

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { platform.time.now_ms };\n\
: main ( -- i64 )\n\
  platform.time.now_ms drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .env("LANG_SYSROOT", &bad_sysroot)
        .args([
            "--emit=ir",
            &format!("--sysroot={}", repo_sysroot().to_string_lossy()),
            "Main.mod",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn milestone2_nested_imports_compile_each_unit() {
    build_tools();
    let dir = fresh_dir("milestone2_nested_imports_compile_each_unit");

    // Leaf module B.
    std::fs::write(
        dir.join("B.def"),
        b"module B;\nexport { x };\n: x ( -- i64 ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("B.mod"),
        b"module B;\nexport { x };\n: x ( -- i64 ) 7 ;\nend;\n",
    )
    .unwrap();

    // Module A depends on B.
    std::fs::write(
        dir.join("A.def"),
        b"module A;\nexport { y };\n: y ( -- i64 ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("A.mod"),
        b"module A;\nimport B { x };\nexport { y };\n: y ( -- i64 ) x ;\nend;\n",
    )
    .unwrap();

    // Compile A as its own unit; this must resolve B via the same search path rules.
    let out_a = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "A.mod"])
        .output()
        .unwrap();
    assert!(
        out_a.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_a.stderr)
    );

    // Compile Main importing A.
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport A { y };\n: main ( -- i64 ) y ;\nend;\n",
    )
    .unwrap();
    let out_main = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out_main.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_main.stderr)
    );
}
