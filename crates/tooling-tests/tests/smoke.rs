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

#[test]
fn milestone3_cast_trunc_u8_runtime() {
    build_tools();
    let dir = fresh_dir("milestone3_cast_trunc_u8_runtime");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  256 as u8 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone3_cast_sign_i8_runtime() {
    build_tools();
    let dir = fresh_dir("milestone3_cast_sign_i8_runtime");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  -1 as i8 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(255));
}

#[test]
fn milestone3_bitcast_size_mismatch_rejected() {
    build_tools();
    let dir = fresh_dir("milestone3_bitcast_size_mismatch_rejected");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  1 bitcast u32 drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3304]"));
}

#[test]
fn milestone3_raw_pointer_casts_gated_by_flag() {
    build_tools();
    let dir = fresh_dir("milestone3_raw_pointer_casts_gated_by_flag");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  0 as usize as ptr drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out_no = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out_no.status.success());
    let stderr = String::from_utf8_lossy(&out_no.stderr);
    assert!(stderr.contains("error[E3305]"));

    let out_yes = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--allow-raw-casts", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out_yes.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_yes.stderr)
    );
}

#[test]
fn milestone4_contract_fail_traps_with_code() {
    build_tools();
    let dir = fresh_dir("milestone4_contract_fail_traps_with_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  requires [ false ]\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(20));
}

#[test]
fn milestone4_subtype_fail_traps_with_code() {
    build_tools();
    let dir = fresh_dir("milestone4_subtype_fail_traps_with_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
subtype Percent = i64 range 0..100;\n\
: main ( -- i64 )\n\
  200 as Percent drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--checks=all", "--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(21));
}

#[test]
fn milestone4_trap_loc_emission_under_g() {
    build_tools();
    let dir = fresh_dir("milestone4_trap_loc_emission_under_g");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
subtype Percent = i64 range 0..0;\n\
: main ( -- i64 )\n\
  1 as Percent drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["-g", "--checks=all", "--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let asm = String::from_utf8_lossy(&out.stdout);
    assert!(asm.contains("__lang_trap_loc:"));
    assert!(asm.contains("jmp __lang_trap_loc"));
}

#[test]
fn milestone5_region_scoped_borrow_must_be_consumed() {
    build_tools();
    let dir = fresh_dir("milestone5_region_scoped_borrow_must_be_consumed");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( Region -- Region )\n\
  &[\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3506]"));
}

#[test]
fn milestone5_region_scoped_borrow_drop_ok() {
    build_tools();
    let dir = fresh_dir("milestone5_region_scoped_borrow_drop_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( Region -- Region )\n\
  &[\n\
    drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
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
fn milestone5_scoped_borrow_requires_array_or_region() {
    build_tools();
    let dir = fresh_dir("milestone5_scoped_borrow_requires_array_or_region");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( i64 -- i64 )\n\
  &[\n\
    drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3515]"));
}

#[test]
fn milestone5_mut_scoped_borrow_forbids_suspend() {
    build_tools();
    let dir = fresh_dir("milestone5_mut_scoped_borrow_forbids_suspend");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { platform.task.yield };\n\
: f ( Region -- Region )\n\
  &![\n\
    platform.task.yield\n\
    drop\n\
  ]\n\
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
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"));
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
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("module Main"));
    assert!(stdout.contains("word pick"));
    assert!(stdout.contains("word countdown"));
    assert!(stdout.contains("block b"));
    assert!(stdout.matches("br_if").count() >= 2, "stdout:\n{stdout}");
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
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cast"));
    assert!(stdout.contains("cmp_ge"));
    assert!(stdout.contains("cmp_le"));
    assert!(stdout.contains("trap_if_false SUBTYPE_FAIL"));
    assert!(stdout.contains("trap_if_false CONTRACT_FAIL"));
}

#[test]
fn opcode_ir_coverage_basic_ops() {
    build_tools();
    let dir = fresh_dir("opcode_ir_coverage_basic_ops");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  1 dup swap drop drop\n\
  2 3 + 4 - 5 * drop\n\
  true false and true or not drop\n\
  0 as usize as ptr_mut 42 !i64\n\
  0 as usize as ptr @i64 drop\n\
  1 1 == [ 0 ] [ 1 ] if\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--allow-raw-casts", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for needle in [
        "dup",
        "swap",
        "drop",
        "add_i64",
        "sub_i64",
        "mul_i64",
        "and_bool",
        "or_bool",
        "not_bool",
        "cmp_eq",
        "br_if",
        "load i64",
        "store i64",
    ] {
        assert!(stdout.contains(needle), "missing {needle} in IR");
    }
}

#[test]
fn opcode_asm_coverage_basic_ops() {
    build_tools();
    let dir = fresh_dir("opcode_asm_coverage_basic_ops");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  1 dup swap drop drop\n\
  2 3 + 4 - 5 * drop\n\
  true false and true or not drop\n\
  0 as usize as ptr_mut 42 !i64\n\
  0 as usize as ptr @i64 drop\n\
  1 1 == [ 0 ] [ 1 ] if\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "--allow-raw-casts", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn milestone6_mmio_volatile_ops_in_ir() {
    build_tools();
    let dir = fresh_dir("milestone6_mmio_volatile_ops_in_ir");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x00 OUT_SET u32 wo volatile\n\
  0x20 IN      u32 ro volatile\n\
end;\n\
const gpio = GPIO @ 0x1000;\n\
: read_in ( -- u32 )\n\
  &gpio.IN @u32\n\
;\n\
: set_out ( u32 -- )\n\
  &!gpio.OUT_SET swap !u32\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("vol_load u32 gpio.IN"));
    assert!(stdout.contains("vol_store u32 gpio.OUT_SET"));
}

#[test]
fn milestone6_mmio_field_address_forbidden() {
    build_tools();
    let dir = fresh_dir("milestone6_mmio_field_address_forbidden");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map UART\n\
  0x00 CTRL u32 rw volatile { enable 0 bool rw }\n\
end;\n\
const uart = UART @ 0x2000;\n\
: bad ( -- )\n\
  &uart.CTRL.enable drop\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3608]"));
}

#[test]
fn milestone6_mmio_alignment_error() {
    build_tools();
    let dir = fresh_dir("milestone6_mmio_alignment_error");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map Bad\n\
  0x01 X u32 rw volatile\n\
end;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3611]"));
}

#[test]
fn milestone6_mmio_access_mode_violations() {
    build_tools();
    let dir = fresh_dir("milestone6_mmio_access_mode_violations");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x00 OUT u32 wo volatile\n\
  0x04 IN  u32 ro volatile\n\
end;\n\
const gpio = GPIO @ 0x1000;\n\
: bad_read ( -- u32 )\n\
  &gpio.OUT @u32\n\
;\n\
: bad_write ( u32 -- )\n\
  &!gpio.IN swap !u32\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3610]") || stderr.contains("error[E3609]"));
}

#[test]
fn milestone6_mmio_array_bounds_error() {
    build_tools();
    let dir = fresh_dir("milestone6_mmio_array_bounds_error");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x200 PINCFG[2] u32 rw volatile\n\
end;\n\
const gpio = GPIO @ 0x1000;\n\
: bad ( -- u32 )\n\
  &gpio.PINCFG'2 @u32\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3604]"));
}

#[test]
fn milestone6_mmio_simulated_rw_u32_runtime() {
    build_tools();
    let dir = fresh_dir("milestone6_mmio_simulated_rw_u32_runtime");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x10 DATA u32 rw volatile\n\
end;\n\
const gpio = GPIO @ 0x100;\n\
: main ( -- i64 )\n\
  42 as u32 &!gpio.DATA swap !u32\n\
  &gpio.DATA @u32 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(42));
}

#[test]
fn milestone6_mmio_simulated_field_store_load_runtime() {
    build_tools();
    let dir = fresh_dir("milestone6_mmio_simulated_field_store_load_runtime");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map UART\n\
  0x00 CTRL u32 rw volatile { mode 0..7 u8 rw flag 8 bool rw }\n\
end;\n\
const uart = UART @ 0x200;\n\
: main ( -- i64 )\n\
  uart.CTRL.mode 7 as u8 !\n\
  uart.CTRL.flag true !\n\
  uart.CTRL.mode @ as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(7));
}

#[test]
fn milestone7_compile_assemble_run_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone7_compile_assemble_run_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 )\n  1 2 +\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(3));
}

fn runtime_asm_linux_x86_64_hosted() -> PathBuf {
    workspace_root()
        .join("runtime")
        .join("linux-x86_64-hosted.asm")
}

#[test]
fn milestone7_emit_obj_link_run_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone7_emit_obj_link_run_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 )\n  7\n;\nend;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "Main.mod",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(7));
}

#[test]
fn milestone7_emit_obj_link_trap_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone7_emit_obj_link_trap_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nsubtype Percent = i64 range 0..100;\n: main ( -- i64 )\n  -1 as Percent drop\n  0\n;\nend;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--checks=all",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
            "Main.mod",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(21));
}

#[test]
fn milestone7_if_while_locals_smoke() {
    build_tools();
    let dir = fresh_dir("milestone7_if_while_locals_smoke");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  0\n\
  [ dup 3 < ] [ 1 + ] while\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(3));
}

#[test]
fn milestone10_golden_asm_add() {
    build_tools();
    let dir = fresh_dir("milestone10_golden_asm_add");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 )\n  1 2 +\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    fn norm(s: &str) -> String {
        let mut out = String::new();
        for part in s.split_inclusive('\n') {
            let (line, nl) = if let Some(stripped) = part.strip_suffix('\n') {
                (stripped, "\n")
            } else {
                (part, "")
            };
            out.push_str(line.trim_start());
            out.push_str(nl);
        }
        out
    }
    let expected = "\
format ELF64 executable\n\
entry __lang_start\n\
\n\
segment readable executable\n\
__lang_start:\n\
  mov r15, __lang_ds_base\n\
  mov r14, __lang_ds_limit\n\
  call w_6d61696e\n\
  sub r15, 8\n\
  mov rdi, [r15]\n\
  and rdi, 0xff\n\
  mov rax, 60\n\
  syscall\n\
\n\
__lang_trap:\n\
  mov rax, 60\n\
  syscall\n\
\n\
__stack_overflow:\n\
  mov rdi, 10\n\
  jmp __lang_trap\n\
\n\
w_6d61696e:\n\
  sub rsp, 16\n\
  jmp .b0_0\n\
.b0_0:\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov qword [r15], 1\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_0\n\
  mov [__lang_ds_high], r15\n\
.ds_high_0:\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov qword [r15], 2\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_1\n\
  mov [__lang_ds_high], r15\n\
.ds_high_1:\n\
  sub r15, 8\n\
  mov rcx, [r15]\n\
  sub r15, 8\n\
  mov rax, [r15]\n\
  add rax, rcx\n\
  mov [r15], rax\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_2\n\
  mov [__lang_ds_high], r15\n\
.ds_high_2:\n\
  sub r15, 8\n\
  mov rax, [r15]\n\
  mov [rsp+8], rax\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov rax, [rsp+8]\n\
  mov [r15], rax\n\
  add r15, 8\n\
  cmp r15, [__lang_ds_high]\n\
  jna .ds_high_3\n\
  mov [__lang_ds_high], r15\n\
.ds_high_3:\n\
  jmp .endword_0\n\
.endword_0:\n\
  add rsp, 16\n\
  ret\n\
\n\
segment readable writeable\n\
__lang_ds_base rb 65536\n\
__lang_ds_limit:\n\
__lang_ds_high dq 0\n";

    assert_eq!(norm(&stdout), norm(expected));
}

#[test]
fn milestone11_contract_failure_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone11_contract_failure_traps_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: bad ( -- ) requires [ false ] ;\n\
: main ( -- i64 )\n\
  bad\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--checks=contracts", "--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(20));
}

#[test]
fn milestone11_subtype_failure_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone11_subtype_failure_traps_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
subtype Percent = i64 range 0..100;\n\
: main ( -- i64 )\n\
  -1 as Percent\n\
  drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--checks=all", "--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(21));
}

#[test]
fn milestone11_stack_overflow_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone11_stack_overflow_traps_exit_code");

    std::fs::write(
        dir.join("Main.asm"),
        b"format ELF64 executable\n\
entry __lang_start\n\
\n\
segment readable executable\n\
__lang_start:\n\
  mov r15, __lang_ds_base\n\
  mov r14, __lang_ds_limit\n\
.loop:\n\
  lea rax, [r15+8]\n\
  cmp rax, r14\n\
  ja __stack_overflow\n\
  mov qword [r15], 0\n\
  add r15, 8\n\
  jmp .loop\n\
\n\
__lang_trap:\n\
  mov rax, 60\n\
  syscall\n\
\n\
__stack_overflow:\n\
  mov rdi, 10\n\
  jmp __lang_trap\n\
\n\
segment readable writeable\n\
__lang_ds_base rb 65536\n\
__lang_ds_limit:\n",
    )
    .unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(10));
}

#[test]
fn milestone12_platform_io_log_writes_stderr() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_io_log_writes_stderr");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  \"hi\\n\" platform.io.log\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).output().unwrap();
    assert_eq!(run.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("hi\n"), "stderr: {stderr:?}");
}

#[test]
fn milestone12_platform_time_now_ms_runs() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_time_now_ms_runs");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  platform.time.now_ms drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone12_platform_critical_noop_runs() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_critical_noop_runs");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  platform.critical.enter\n\
  platform.critical.exit\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone12_platform_task_sleep_runs() {
    build_tools();
    let dir = fresh_dir("milestone12_platform_task_sleep_runs");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 ) !{suspend}\n\
  0 as usize platform.task.sleep-ms\n\
  0 as usize platform.task.sleep-us\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
        ])
        .arg(&sysroot_arg)
        .arg("Main.mod")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone13_array_type_and_scoped_slice_typechecks() {
    build_tools();
    let dir = fresh_dir("milestone13_array_type_and_scoped_slice_typechecks");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( i64'4 -- i64'4 )\n\
  &[\n\
    drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("i64'4"));
    assert!(stdout.contains("Slice(i64)"));
    assert!(stdout.contains("scoped_enter"));
}

#[test]
fn milestone13_borrow_destructuring_emits_ptr_offsets() {
    build_tools();
    let dir = fresh_dir("milestone13_borrow_destructuring_emits_ptr_offsets");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i64\n\
  y : i64\n\
end;\n\
resource r : Point;\n\
: f ( -- )\n\
  r lock [\n\
    &r => { &x &y }\n\
    x drop\n\
    y drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ptr_add_const"), "stdout: {stdout}");
}

#[test]
fn milestone13_borrowed_slice_cannot_escape_via_return() {
    build_tools();
    let dir = fresh_dir("milestone13_borrowed_slice_cannot_escape_via_return");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( i64'4 -- Slice(i64) )\n\
  &[\n\
    swap drop\n\
    return\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3511]"), "stderr: {stderr}");
}

#[test]
fn milestone13_borrowed_slice_live_in_local_blocks_yield() {
    build_tools();
    let dir = fresh_dir("milestone13_borrowed_slice_live_in_local_blocks_yield");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: f ( i64'4 -- i64'4 ) !{suspend}\n\
  &[\n\
    => s\n\
    platform.task.yield\n\
    s drop\n\
  ]\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3502]"), "stderr: {stderr}");
}

#[test]
fn milestone14_borrow_resource_outside_lock_fails() {
    build_tools();
    let dir = fresh_dir("milestone14_borrow_resource_outside_lock_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  &!counter drop\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5004]"), "stderr: {stderr}");
}

#[test]
fn milestone14_borrow_resource_inside_lock_ok() {
    build_tools();
    let dir = fresh_dir("milestone14_borrow_resource_inside_lock_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  counter [ &!counter drop ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
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
fn milestone14_shared_borrow_resource_inside_lock_ok() {
    build_tools();
    let dir = fresh_dir("milestone14_shared_borrow_resource_inside_lock_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  counter [ &counter drop ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
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
fn milestone14_borrow_other_resource_inside_lock_fails() {
    build_tools();
    let dir = fresh_dir("milestone14_borrow_other_resource_inside_lock_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
resource other : i64;\n\
: f ( -- )\n\
  counter [ &!other drop ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5004]"), "stderr: {stderr}");
}

#[test]
fn milestone14_nested_lock_rejected() {
    build_tools();
    let dir = fresh_dir("milestone14_nested_lock_rejected");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
resource counter : i64;\n\
: f ( -- )\n\
  counter [\n\
    counter [ ] lock\n\
  ] lock\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5002]"), "stderr: {stderr}");
}

#[test]
fn milestone15_struct_field_borrow_and_load_ok() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_field_borrow_and_load_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
  y : i32\n\
end;\n\
: getx ( Point -- i32 )\n\
  => p\n\
  &p.x @i32\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
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
fn milestone15_struct_ptr_field_access_emits_ptr_add_const() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_ptr_field_access_emits_ptr_add_const");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
  y : i32\n\
end;\n\
: getx ( Point -- i32 )\n\
  => p\n\
  &p ->x @i32\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ptr_add_const"), "stdout: {stdout}");
}

#[test]
fn milestone15_struct_field_unknown_field_fails() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_field_unknown_field_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
end;\n\
: bad ( Point -- )\n\
  => p\n\
  &p.z drop\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3716]"), "stderr: {stderr}");
}

#[test]
fn milestone15_struct_field_load_type_mismatch_fails() {
    build_tools();
    let dir = fresh_dir("milestone15_struct_field_load_type_mismatch_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
struct Point\n\
  x : i32\n\
end;\n\
: bad ( Point -- bool )\n\
  => p\n\
  &p.x @bool\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3717]"), "stderr: {stderr}");
}

#[test]
fn milestone15_enum_variant_literal_ok() {
    build_tools();
    let dir = fresh_dir("milestone15_enum_variant_literal_ok");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
enum State : u8\n\
  Idle = 0x00\n\
  Run  = 0x01\n\
end;\n\
: f ( -- State )\n\
  State.Idle\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
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
fn milestone15_unknown_enum_variant_fails() {
    build_tools();
    let dir = fresh_dir("milestone15_unknown_enum_variant_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
enum State : u8\n\
  Idle = 0\n\
end;\n\
: f ( -- State )\n\
  State.Missing\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3725]"), "stderr: {stderr}");
}

#[test]
fn milestone16_channel_send_recv_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_send_recv_roundtrip_exit_code");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make => ch\n\
  ch 42 |>\n\
  ch <| 42 == [ 0 ] [ 1 ] if\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone16_channel_two_channels_independent_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_two_channels_independent_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make => ch1\n\
  platform.channel.make => ch2\n\
  ch1 40 |>\n\
  ch2 2 |>\n\
  ch1 <| ch2 <| +\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(42));
}

#[test]
fn milestone16_channel_send_type_mismatch_fails() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_send_type_mismatch_fails");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: bad ( |i64| -- )\n\
  => ch\n\
  ch true |>\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3732]"), "stderr: {stderr}");
}

#[test]
fn milestone16_channel_task_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_task_roundtrip_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
: main ( -- i64 ) !{suspend}\n\
  platform.channel.make drop\n\
  [ ( -- ) ] platform.task.spawn => t\n\
  0 bitcast |Task| t |>\n\
  0 bitcast |Task| <| platform.task.join\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone16_channel_send_blocks_on_full() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_send_blocks_on_full");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
register-map GPIO\n\
  0x00 DATA[2] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) !{suspend}\n\
  platform.channel.make drop\n\
  0 as u32 &!gpio.DATA'0 swap !u32\n\
  0 as u32 &!gpio.DATA'1 swap !u32\n\
  0\n\
  [ dup 64 < ]\n\
  [ dup 0 bitcast |i64| swap |> 1 + ] while\n\
  drop\n\
  [ ( -- )\n\
    &!gpio.DATA'0 @u32 as i64 1 == [ 1 as u32 &!gpio.DATA'1 swap !u32 ] [ ] if\n\
    0 bitcast |i64| <| drop\n\
  ] platform.task.spawn\n\
  0 bitcast |i64| 99 |>\n\
  1 as u32 &!gpio.DATA'0 swap !u32\n\
  platform.task.join\n\
  &gpio.DATA'1 @u32 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn mmio_array_const_index_emits_ptr_add_const() {
    build_tools();
    let dir = fresh_dir("mmio_array_const_index_emits_ptr_add_const");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x00 DATA[4] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 )\n\
  gpio.DATA'2 @u32 drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ptr_add_const"), "stdout: {stdout}");
}

#[test]
fn mmio_array_dynamic_index_emits_ptr_add_index() {
    build_tools();
    let dir = fresh_dir("mmio_array_dynamic_index_emits_ptr_add_index");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
register-map GPIO\n\
  0x00 DATA[4] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 )\n\
  1 => idx\n\
  gpio.DATA'(idx) @u32 drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ptr_add_index"), "stdout: {stdout}");
}

#[test]
fn milestone16_channel_recv_blocks_on_empty() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_recv_blocks_on_empty");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
register-map GPIO\n\
  0x00 DATA[2] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) !{suspend}\n\
  platform.channel.make drop\n\
  0 as u32 &!gpio.DATA'0 swap !u32\n\
  0 as u32 &!gpio.DATA'1 swap !u32\n\
  [ ( -- )\n\
    &!gpio.DATA'0 @u32 as i64 1 == [ 1 as u32 &!gpio.DATA'1 swap !u32 ] [ ] if\n\
    0 bitcast |i64| 123 |>\n\
  ] platform.task.spawn\n\
  0 bitcast |i64| <| drop\n\
  1 as u32 &!gpio.DATA'0 swap !u32\n\
  platform.task.join\n\
  &gpio.DATA'1 @u32 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone16_channel_deadlock_traps_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_channel_deadlock_traps_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make drop\n\
  0 bitcast |i64| <| drop\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(23));
}

#[test]
fn milestone8_sysroot_flag_allows_imports() {
    build_tools();
    let dir = fresh_dir("milestone8_sysroot_flag_allows_imports");
    let sysroot = dir.join("sysroot");
    std::fs::create_dir_all(sysroot.join("platform")).unwrap();

    std::fs::write(
        sysroot.join("Core.def"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("Core.mod"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("platform").join("linux.def"),
        b"module platform/linux;\n: platform.io.log ( str -- ) ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("platform").join("linux.mod"),
        b"module platform/linux;\n: platform.io.log ( str -- ) drop ;\nend;\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { };\nimport platform/linux { };\n: main ( -- i64 )\n  \"hi\" platform.io.log\n  0\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", sysroot.to_string_lossy()))
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("platform.io.log"));
}

#[test]
fn milestone8_sysroot_iface_mismatch_fails() {
    build_tools();
    let dir = fresh_dir("milestone8_sysroot_iface_mismatch_fails");
    let sysroot = dir.join("sysroot");
    std::fs::create_dir_all(sysroot.join("platform")).unwrap();

    std::fs::write(
        sysroot.join("Core.def"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("Core.mod"),
        b"module Core;\nexport { };\nend;\n",
    )
    .unwrap();
    std::fs::write(
        sysroot.join("platform").join("linux.def"),
        b"module platform/linux;\n: platform.io.log ( str -- ) ;\nend;\n",
    )
    .unwrap();
    // mismatch: wrong signature type
    std::fs::write(
        sysroot.join("platform").join("linux.mod"),
        b"module platform/linux;\n: platform.io.log ( i64 -- ) drop ;\nend;\n",
    )
    .unwrap();

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport platform/linux { };\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", sysroot.to_string_lossy()))
        .args(["--emit=ast", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E2218]"));
}

#[test]
fn milestone8_effect_non_suspend_cannot_yield() {
    build_tools();
    let dir = fresh_dir("milestone8_effect_non_suspend_cannot_yield");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  platform.task.yield\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"), "stderr: {stderr}");
}

#[test]
fn milestone8_effect_suspend_word_allows_yield_runs() {
    build_tools();
    let dir = fresh_dir("milestone8_effect_suspend_word_allows_yield_runs");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 ) !{suspend}\n\
  platform.task.yield\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
        ])
        .arg(&sysroot_arg)
        .arg("Main.mod")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone8_platform_task_run_allows_yield_in_quote() {
    build_tools();
    let dir = fresh_dir("milestone8_platform_task_run_allows_yield_in_quote");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { };\n\
: main ( -- i64 )\n\
  [ platform.task.yield ] platform.task.run\n\
  0\n\
;\n\
end;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
        ])
        .arg(&sysroot_arg)
        .arg("Main.mod")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn task_call_allows_escaping_quote_body() {
    build_tools();
    let dir = fresh_dir("task_call_allows_escaping_quote_body");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
: main ( -- i64 )\n\
  41 [ ( i64 -- i64 ) 1 + ] call\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(42));
}

#[test]
fn task_spawn_allows_escaping_quote_body() {
    build_tools();
    let dir = fresh_dir("task_spawn_allows_escaping_quote_body");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { platform.task.spawn, platform.task.join };\n\
register-map GPIO\n\
  0x00 DATA u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) !{suspend}\n\
  [ ( -- ) 7 as u32 &!gpio.DATA swap !u32 ] platform.task.spawn\n\
  platform.task.join\n\
  &gpio.DATA @u32 as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(7));
}

#[test]
fn task_scheduler_stress_many_tasks() {
    build_tools();
    let dir = fresh_dir("task_scheduler_stress_many_tasks");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/linux { platform.task.spawn, platform.task.join, platform.task.yield };\n\
register-map GPIO\n\
  0x00 DATA[8] u32 rw\n\
end;\n\
const gpio = GPIO @ 0x0;\n\
: main ( -- i64 ) !{suspend}\n\
  [ ( -- ) !{suspend} platform.task.yield 1 as u32 &!gpio.DATA'0 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) !{suspend} platform.task.yield 2 as u32 &!gpio.DATA'1 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) !{suspend} platform.task.yield 3 as u32 &!gpio.DATA'2 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) !{suspend} platform.task.yield 4 as u32 &!gpio.DATA'3 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) !{suspend} platform.task.yield 5 as u32 &!gpio.DATA'4 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) !{suspend} platform.task.yield 6 as u32 &!gpio.DATA'5 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) !{suspend} platform.task.yield 7 as u32 &!gpio.DATA'6 swap !u32 ] platform.task.spawn\n\
  [ ( -- ) !{suspend} platform.task.yield 8 as u32 &!gpio.DATA'7 swap !u32 ] platform.task.spawn\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  platform.task.join\n\
  &gpio.DATA'0 @u32 as i64\n\
  &gpio.DATA'1 @u32 as i64 +\n\
  &gpio.DATA'2 @u32 as i64 +\n\
  &gpio.DATA'3 @u32 as i64 +\n\
  &gpio.DATA'4 @u32 as i64 +\n\
  &gpio.DATA'5 @u32 as i64 +\n\
  &gpio.DATA'6 @u32 as i64 +\n\
  &gpio.DATA'7 @u32 as i64 +\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .arg(&sysroot_arg)
        .args(["--emit=asm", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(dir.join("Main.asm"), &out.stdout).unwrap();

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args(["--out=prog", "Main.asm"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(36));
}

#[test]
fn milestone8_typed_channel_u32_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone8_typed_channel_u32_roundtrip_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make bitcast |u32| => ch\n\
  ch 42 as u32 |>\n\
  ch <| as i64\n\
;\n\
end;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
        ])
        .arg(&sysroot_arg)
        .arg("Main.mod")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(42));
}

#[test]
fn milestone16_typed_channel_i8_roundtrip_exit_code() {
    build_tools();
    let dir = fresh_dir("milestone16_typed_channel_i8_roundtrip_exit_code");
    let sysroot_arg = format!("--sysroot={}", repo_sysroot().to_string_lossy());

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
import platform/channel { };\n\
: main ( -- i64 )\n\
  platform.channel.make drop\n\
  0 bitcast |i8| -1 as i8 |>\n\
  0 bitcast |i8| <| as i64 -1 == [ 0 ] [ 1 ] if\n\
;\n\
end;\n",
    )
    .unwrap();

    let status = Command::new(exe("langc"))
        .current_dir(&dir)
        .args([
            "--emit=obj",
            "--target=x86_64-unknown-linux-gnu",
            "--out-dir=.",
        ])
        .arg(&sysroot_arg)
        .arg("Main.mod")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(exe("lang-assemble"))
        .current_dir(&dir)
        .args([
            "--out=rt.o",
            runtime_asm_linux_x86_64_hosted().to_string_lossy().as_ref(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new("ld")
        .current_dir(&dir)
        .args(["-o", "prog", "rt.o", "Main.o"])
        .status()
        .unwrap();
    assert!(status.success());

    let run = Command::new(dir.join("prog")).status().unwrap();
    assert_eq!(run.code(), Some(0));
}

#[test]
fn milestone8_iso_dup_forbidden() {
    build_tools();
    let dir = fresh_dir("milestone8_iso_dup_forbidden");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
iso Msg;\n\
: bad_dup ( Msg -- Msg Msg )\n\
  dup\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5010]"), "stderr: {stderr}");
}

#[test]
fn milestone8_iso_drop_forbidden() {
    build_tools();
    let dir = fresh_dir("milestone8_iso_drop_forbidden");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
iso Msg;\n\
: bad_drop ( Msg -- )\n\
  drop\n\
;\n\
end;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5011]"), "stderr: {stderr}");
}

#[test]
fn milestone9_golden_ir_dump_if_while_locals() {
    build_tools();
    let dir = fresh_dir("milestone9_golden_ir_dump_if_while_locals");

    std::fs::write(dir.join("Core.def"), b"module Core;\nexport { };\nend;\n").unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { };\n: pick ( i64 i64 bool -- i64 )\n  [ drop ] [ swap drop ] if\n;\n: countdown ( i64 -- i64 )\n  [ dup 0 > ] [ 1 - ] while\n;\n: locals ( i64 -- i64 )\n  => x\n  x\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    // Keep the golden minimal but stable: ensure blocks/branches/locals are present.
    assert!(stdout.contains("word pick"));
    assert!(stdout.contains("br_if"));
    assert!(stdout.contains("word countdown"));
    assert!(stdout.contains("local_set"));
    assert!(stdout.contains("local_get"));
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
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("trap_if_false CONTRACT_FAIL"));
    assert!(!stdout.contains("SUBTYPE_FAIL"));

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--checks=off", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("CONTRACT_FAIL"));
    assert!(!stdout.contains("SUBTYPE_FAIL"));
}

#[test]
fn milestone5_rejects_mutable_borrow_of_local() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_mutable_borrow_of_local");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64 -- ptr_mut )\n  => x\n  &!x\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3501]"));
}

#[test]
fn milestone5_scoped_borrow_must_be_consumed() {
    build_tools();
    let dir = fresh_dir("milestone5_scoped_borrow_must_be_consumed");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64'1 -- i64'1 )\n  &[\n  ]\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3506]"));
}

#[test]
fn milestone5_rejects_suspend_with_scoped_live() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_suspend_with_scoped_live");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64'1 -- i64'1 ) !{suspend}\n  &[\n    platform.task.yield drop\n  ]\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E3502]"));
}

#[test]
fn milestone5_rejects_suspend_inside_mut_scoped_block() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_suspend_inside_mut_scoped_block");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64'1 -- i64'1 )\n  &![\n    drop platform.task.yield\n  ]\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"));
}

#[test]
fn milestone5_rejects_suspend_inside_lock() {
    build_tools();
    let dir = fresh_dir("milestone5_rejects_suspend_inside_lock");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64 -- i64 )\n  [ platform.task.yield ] lock\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[E5001]"));
}

#[test]
fn milestone5_allows_drop_before_yield() {
    build_tools();
    let dir = fresh_dir("milestone5_allows_drop_before_yield");

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: f ( i64'1 -- i64'1 ) !{suspend}\n  &[\n    drop\n  ]\n  platform.task.yield\n;\nend;\n",
    )
    .unwrap();

    let out = Command::new(exe("langc"))
        .current_dir(&dir)
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
fn langc_x86_64_unknown_none_target_recognized() {
    build_tools();
    let dir = fresh_dir("langc_x86_64_none_target");
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n: main ( -- i64 ) 0 ;\nend;\n",
    )
    .unwrap();
    let out = Command::new(exe("langc"))
        .current_dir(&dir)
        .args(["--emit=ir", "--target=x86_64-unknown-none", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
