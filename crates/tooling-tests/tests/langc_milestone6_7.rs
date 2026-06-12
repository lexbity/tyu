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
        b"module Main;\nsubtype Percent = i64 range 0..100;\n: clamp ( i64 -- Percent )\n  as Percent\n;\n: pwm_set ( Percent -- )\n  needs [ dup dup 0 >= swap 100 <= and ]\n  drop\n;\nend;\n",
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
  &gpio.PINCFG.2 @u32\n\
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