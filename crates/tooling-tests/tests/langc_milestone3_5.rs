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

// ---------------------------------------------------------------------------
// Phase 5 — debug_trap_loc emits full 64-bit word_hash and assemblable FASM
// ---------------------------------------------------------------------------

#[test]
fn cur_word_id_is_full_64bit_not_truncated() {
    // Verify that the FNV-1a hash of every common word name has non-zero
    // upper 32 bits, so the old `as u32` truncation (now removed) was
    // semantically wrong for all of them.
    let words = [
        "main", "f", "g", "helper", "test", "trigger-trap",
        "platform.task.yield", "testio.write-byte",
    ];
    for word in &words {
        let hash = fnv1a_u64(word.as_bytes());
        let truncated = hash as u32 as u64;
        assert_ne!(
            hash, truncated,
            "fnv1a_u64(\"{}\") = {:#x} fits in 32 bits — \
             the truncation was immaterial for this word",
            word, hash,
        );
    }
}

fn fnv1a_u64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn write_u64_hex_compact(v: u64, buf: &mut [u8; 18]) -> &[u8] {
    // Produces the same format as codegen_x86_64::util::write_u64_hex:
    // "0x" + lowercase hex with no leading zeros (except "0x0" for zero).
    buf[0] = b'0';
    buf[1] = b'x';
    let mut n = 2usize;
    let mut started = false;
    for i in (0..64).step_by(4).rev() {
        let nib = ((v >> i) & 0xf) as u8;
        if started || nib != 0 || i == 0 {
            started = true;
            buf[n] = match nib {
                0..=9 => b'0' + nib,
                _ => b'a' + (nib - 10),
            };
            n += 1;
        }
    }
    &buf[..n]
}

#[test]
fn debug_trap_loc_emits_64bit_word_hash() {
    build_tools();
    let dir = fresh_dir("debug_trap_loc_hash");

    // A fixture that triggers a subtype trap under --checks=all.
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\n\
         subtype Small = i64 range 0..10;\n\
         : main ( -- i64 )\n\
           100 as Small drop\n\
           0 ;\n\
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
        "langc -g --emit=asm failed:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Write the assembly to a file and assert fasm (the x86 assembler)
    // can assemble it — the fixed output must be valid FASM syntax.
    let asm_path = dir.join("Main.asm");
    std::fs::write(&asm_path, &out.stdout).unwrap();
    let fasm_status = Command::new("fasm")
        .current_dir(&dir)
        .args(["Main.asm", "Main_bin"])
        .status()
        .unwrap();
    assert!(
        fasm_status.success(),
        "fasm failed on -g output:\n{}\n--- stdout ---\n{}\n--- stderr ---",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stdout),
    );

    let asm = String::from_utf8_lossy(&out.stdout);

    // The trap site must carry the full 64-bit fnv1a_u64("main"),
    // not a truncated 32-bit value.  The emitter uses write_u64_hex
    // which produces "0x<lowercase-hex>".
    let main_hash = fnv1a_u64(b"main");
    let mut hash_buf = [0u8; 18];
    let hash_str = write_u64_hex_compact(main_hash, &mut hash_buf);
    let hash_pattern = format!("mov rcx, {}", core::str::from_utf8(hash_str).unwrap());

    assert!(
        asm.contains(&hash_pattern),
        "trap site must carry full 64-bit hash '{}' for word 'main' (hash={:#x}).\n\
         Assembly output:\n{}",
        hash_pattern,
        main_hash,
        asm,
    );

    // Also verify that the hash is NOT representable as 32-bit all-zeros
    // (a common truncation pattern).  For word "main" the FNV-1a hash
    // has non-zero upper 32 bits, so a 32-bit truncation would be wrong.
    let truncated = main_hash as u32 as u64;
    assert_ne!(
        main_hash, truncated,
        "fnv1a_u64(\"main\") must not be representable in 32 bits \
         (otherwise the truncation wouldn't matter)"
    );
}

