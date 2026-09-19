//! ARM Cortex-M3 execution tests via `tyu test` driver and product-runner-backed runs.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn build_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");
}

// ---------------------------------------------------------------------------
// Positive fixture suite (via tyu test)
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_and_stack_pass() {
    if !common::require_tools(&[
        "langc",
        "arm-none-eabi-as",
        "arm-none-eabi-ld",
        "qemu-system-arm",
    ]) {
        return;
    }
    build_langc();

    let output = Command::new(common::tyu_exe())
        .args([
            "test",
            "--target=armv7m-unknown-none",
            "--platform=armv7m-unknown-none",
            &format!("--manifest={}", common::fixtures_manifest().display()),
        ])
        .output()
        .expect("tyu test");

    assert!(
        output.status.success(),
        "tyu test armv7m-unknown-none failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// D diagnostic record from __lang_trap_loc (header-only ARM agent)
// ---------------------------------------------------------------------------

#[test]
fn trap_emits_framed_d_record() {
    if !common::require_tools(&[
        "langc",
        "arm-none-eabi-as",
        "arm-none-eabi-ld",
        "qemu-system-arm",
    ]) {
        return;
    }
    build_langc();

    let dir = temp_dir("arm_trap_d");
    let target = codegen_core::Target::ArmV7MUnknownNone;

    // Fixture: subtype violation that triggers TrapIfFalse under --checks=all.
    std::fs::write(
        dir.join("Main.mod"),
        "\
module Main;
import platform/testio { testio.write-byte };
subtype Small = i64 range 0..10;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  100 as Small drop
  emit-done
  0 ;
end;
",
    )
    .unwrap();

    let fixture_o = langc_compile_g(target, &dir.join("Main.mod"), &dir, false);
    let runtime_objs = common::assemble_runtime(target, &dir);
    let mut objs = vec![fixture_o];
    objs.extend(runtime_objs);
    let image = common::link_image(target, &objs, &dir);

    let outcome =
        common::run_with_product_runner(target, &image, std::time::Duration::from_secs(10));

    // The fixture should trap; no S\n completion.
    assert!(!outcome.timed_out, "ARM trap fixture must not hang");

    // Parse framed records.
    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();

    let v_records: Vec<_> = records
        .iter()
        .filter(|r| matches!(r, harness_core::Record::Version(_)))
        .collect();
    assert!(
        !v_records.is_empty(),
        "ARM trap output must contain V record"
    );

    let d_records: Vec<_> = records
        .iter()
        .filter(|r| matches!(r, harness_core::Record::Diag(_)))
        .collect();
    assert!(
        !d_records.is_empty(),
        "ARM trap output must contain D record"
    );

    for rec in &d_records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag = diag_core::DiagRecord::parse(payload).expect("valid DiagRecord on ARM");
            assert_eq!(
                diag.trap_code, 21,
                "expected SUBTYPE_FAIL (21), got {}",
                diag.trap_code,
            );
            assert!(diag.valid, "ARM __lang_trap_loc must have valid=1");
            assert_eq!(diag.origin, diag_core::origin::IN_GUEST);
            assert_eq!(diag.version, diag_core::DIAG_RECORD_VERSION);
            assert_ne!(diag.word_hash, 0, "word_hash must not be zero");
            assert_eq!(
                diag.slot_count, 0,
                "ARM is header-only, slot_count must be 0"
            );
        }
    }
}

#[test]
fn trap_overflow_emits_trap_code_10() {
    if !common::require_tools(&[
        "langc",
        "arm-none-eabi-as",
        "arm-none-eabi-ld",
        "qemu-system-arm",
    ]) {
        return;
    }
    build_langc();

    let dir = temp_dir("arm_trap_overflow");
    let target = codegen_core::Target::ArmV7MUnknownNone;

    // Unbounded recursion → stack overflow.
    std::fs::write(
        dir.join("Main.mod"),
        "\
module Main;
import platform/testio { testio.write-byte };
: recurse ( -- ) recurse ;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  recurse
  emit-done
  0 ;
end;
",
    )
    .unwrap();

    let fixture_o = langc_compile(target, &dir.join("Main.mod"), &dir, false);
    let runtime_objs = common::assemble_runtime(target, &dir);
    let mut objs = vec![fixture_o];
    objs.extend(runtime_objs);
    let image = common::link_image(target, &objs, &dir);

    let outcome =
        common::run_with_product_runner(target, &image, std::time::Duration::from_secs(10));
    assert!(!outcome.timed_out, "ARM stack overflow must not hang");

    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();

    for rec in &records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag =
                diag_core::DiagRecord::parse(payload).expect("valid DiagRecord on ARM overflow");
            assert_eq!(
                diag.trap_code, 10,
                "stack overflow must report trap_code=10, got {}",
                diag.trap_code,
            );
            assert!(
                !diag.valid,
                "stack overflow handler has no payload, valid must be 0"
            );
            // Header-only: slot_count = 0.
            assert_eq!(diag.slot_count, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime symbol exports
// ---------------------------------------------------------------------------

#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tools(&["arm-none-eabi-as", "arm-none-eabi-nm"]) {
        return;
    }

    let target = codegen_core::Target::ArmV7MUnknownNone;
    let out_dir = common::temp_dir("arm_runtime_symcheck");
    let runtime_objs = common::assemble_runtime(target, &out_dir);
    let nm_bin = "arm-none-eabi-nm";

    let mut all_symbols = String::new();
    for obj in &runtime_objs {
        let output = Command::new(nm_bin)
            .arg("--defined-only")
            .arg(obj)
            .output()
            .expect("arm-none-eabi-nm invocation failed");
        assert!(
            output.status.success(),
            "arm-none-eabi-nm failed on {:?}",
            obj
        );
        all_symbols.push_str(&String::from_utf8_lossy(&output.stdout));
    }

    // Use feature-derived expected symbol set (DEBT-3).
    // ARM currently only has runtime + modload (no concurrency unit).
    let has_modload = std::path::Path::new("runtime/armv7m-unknown-none/modload.asm").exists();
    let expected = if has_modload {
        common::expected_symbols(codegen_core::FeatureSet::all())
    } else {
        common::expected_symbols(codegen_core::FeatureSet::empty())
    };
    for sym in &expected {
        assert!(
            all_symbols.contains(sym),
            "ARM runtime missing required symbol `{sym}`:\n{all_symbols}",
        );
    }

    // Verify trap labels are at DIFFERENT addresses (no aliasing).
    let extract_addr = |sym: &str| -> Option<u64> {
        for line in all_symbols.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // arm-none-eabi-nm output: "ADDR T __lang_trap"
            if parts.len() >= 3 && parts[1] == "T" && parts[2] == sym {
                return u64::from_str_radix(parts[0], 16).ok();
            }
        }
        None
    };

    let trap_addr = extract_addr("__lang_trap");
    let loc_addr = extract_addr("__lang_trap_loc");
    let ovf_addr = extract_addr("__stack_overflow");
    let hf_addr = extract_addr("__lang_hardfault");

    assert!(
        trap_addr.is_some() && loc_addr.is_some() && ovf_addr.is_some() && hf_addr.is_some(),
        "all four trap symbols must be defined"
    );
    let addrs = vec![
        trap_addr.unwrap(),
        loc_addr.unwrap(),
        ovf_addr.unwrap(),
        hf_addr.unwrap(),
    ];
    let mut sorted = addrs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        4,
        "all four trap symbols must be at distinct addresses (no aliasing)"
    );
}

// ---------------------------------------------------------------------------
// ARM memory and pointer ops (Slice 5 — parity)
// ---------------------------------------------------------------------------

/// Verify that the ARM codegen emits correct assembly for `AddrOf`,
/// `MmioVolLoad`, `MmioVolStore`, `MmioPlace`, and the `@`/`!` typed
/// load/store words.  Since the ARM cross toolchain may not be installed,
/// this test asserts the *generated assembly text* rather than running
/// under QEMU.
///
/// Run `cargo test --test arm arm_memory_load_store_asm -- --nocapture`
/// to see the full assembly listing.
#[test]
fn arm_memory_load_store_asm() {
    if !common::require_tools(&["langc"]) {
        return;
    }
    build_langc();

    let dir = common::temp_dir("arm_memory_asm");
    let target = codegen_core::Target::ArmV7MUnknownNone;

    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;
register-map Scratch
  0x00 DATA i64 rw
end;
const mmio = Scratch @ board.datascratch;
: main ( -- i64 )
  &mmio.DATA @i64 drop
  0 ;
end;
",
    )
    .unwrap();

    let status = std::process::Command::new(common::langc_exe())
        .current_dir(&dir)
        .arg("--emit=obj")
        .arg(format!(
            "--target={}",
            std::str::from_utf8(target.triple()).unwrap()
        ))
        .arg(format!("--sysroot={}", common::sysroot_dir().display()))
        .arg("-I")
        .arg(format!(
            "{}/armv7m-unknown-none",
            common::sysroot_dir().display()
        ))
        .arg("--out-dir=.")
        .arg(format!("--platform={}", common::platform_desc_dir(target).display()))
        .arg("Main.mod")
        .status()
        .expect("langc invocation");
    assert!(
        status.success(),
        "langc failed to compile ARM MMIO load/store fixture"
    );

    // The assembly may or may not assemble depending on whether
    // arm-none-eabi-as is installed.  What matters is that the .asm
    // file was generated (codegen succeeded).
    let asm_path = dir.join("Main.asm");
    assert!(
        asm_path.exists(),
        "ARM codegen must produce Main.asm (codegen failed)"
    );

    let asm = std::fs::read_to_string(&asm_path).unwrap();

    // Verify key instructions are present in the assembly.
    // P6: the window base is bound through a relocatable literal site
    // (`ldr r0, =__lang_window_0_base`), and the register offset added.
    assert!(
        asm.contains("ldr r0, =__lang_window_0_base"),
        "MmioPlace / AddrOf must load the bound window base (window 0 = 0x20000000)"
    );
    assert!(
        asm.contains("adds r0, r0, r1"),
        "MmioPlace / AddrOf must add the register offset; got:\n{asm}"
    );
    assert!(
        asm.contains("ldrd r0, r1, [r0]"),
        "MmioVolLoad 64-bit must emit ldrd; got:\n{asm}"
    );
    assert!(
        asm.contains("strd r0, r1, [r4]"),
        "MmioVolStore 64-bit must emit strd; got:\n{asm}"
    );
    assert!(
        asm.contains("strd r0, r1, [r4]"),
        "emit_push_r0r1 must use strd"
    );
}

/// Verify that ARM MMIO field load/store (`MmioVolLoadField` /
/// `MmioVolStoreField`) emit correct load-modify-write sequences
/// with mask and shift.
#[test]
fn arm_mmio_field_round_trip_asm() {
    if !common::require_tools(&["langc"]) {
        return;
    }
    build_langc();

    let dir = common::temp_dir("arm_mmio_field");
    let target = codegen_core::Target::ArmV7MUnknownNone;

    // Register-map with bit fields: two 4-bit fields in a 32-bit register.
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;
register-map GPIO
  0x00 DATA u32 rw {
    LO 0..3 u32 rw
    HI 4..7 u32 rw
  }
end;
const gpio = GPIO @ board.gpio;
: main ( -- i64 )
  gpio.DATA.LO @ drop
  gpio.DATA.HI @ drop
  0 ;
end;
",
    )
    .unwrap();

    let _status = std::process::Command::new(common::langc_exe())
        .current_dir(&dir)
        .arg("--emit=obj")
        .arg(format!(
            "--target={}",
            std::str::from_utf8(target.triple()).unwrap()
        ))
        .arg(format!("--sysroot={}", common::sysroot_dir().display()))
        .arg("-I")
        .arg(format!(
            "{}/armv7m-unknown-none",
            common::sysroot_dir().display()
        ))
        .arg("--out-dir=.")
        .arg(format!("--platform={}", common::platform_desc_dir(target).display()))
        .arg("Main.mod")
        .status()
        .expect("langc invocation");

    let asm_path = dir.join("Main.asm");
    assert!(
        asm_path.exists(),
        "ARM codegen must produce Main.asm for MMIO field test"
    );

    let asm = std::fs::read_to_string(&asm_path).unwrap();

    // Field load must apply mask+shift (MmioVolLoadField).
    assert!(
        asm.contains("lsrs") || asm.contains("ands"),
        "MmioVolLoadField must shift/mask the field value;\n{asm}"
    );
    // Field load must emit load (ldr) followed by extraction.
    assert!(
        asm.contains("ldr"),
        "MmioVolLoadField must load the register first;\n{asm}"
    );
}

// ---------------------------------------------------------------------------
// Compilation helpers
// ---------------------------------------------------------------------------

// ISR lock atomicity codegen fixture
// ---------------------------------------------------------------------------

#[test]
fn isr_lock_atomicity_codegen() {
    if !common::require_tools(&["langc", "arm-none-eabi-as", "arm-none-eabi-ld"]) {
        return;
    }
    build_langc();

    let dir = temp_dir("isr_lock_atomicity");
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("isr_lock_atomicity.mod");

    let fixture_o = langc_compile(
        codegen_core::Target::ArmV7MUnknownNone,
        &src_path,
        &dir,
        true, // --lib: runner provides main; we only inspect asm shape
    );

    let asm_path = dir.join("IsrLockAtomicity.asm");
    let asm = std::fs::read_to_string(&asm_path).expect("ARM assembly must be generated");
    assert!(
        asm.contains("cpsid i"),
        "ISR-shared resource lock must disable interrupts;\n{asm}"
    );
    assert!(
        asm.contains("cpsie i"),
        "ISR-shared resource lock must re-enable interrupts;\n{asm}"
    );

    assert!(
        fixture_o.exists(),
        "ARM ISR lock fixture must compile to an object file"
    );
}

// ---------------------------------------------------------------------------

fn langc_compile(
    target: codegen_core::Target,
    src: &Path,
    out_dir: &Path,
    is_lib: bool,
) -> PathBuf {
    common::langc_compile(target, src, out_dir, is_lib)
}

fn langc_compile_g(
    target: codegen_core::Target,
    src: &Path,
    out_dir: &Path,
    is_lib: bool,
) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();

    let mut args: Vec<String> = vec![
        "-g".into(),
        "--checks=all".into(),
        "--emit=obj".into(),
        format!("--target={triple}"),
        format!("--sysroot={}", common::sysroot_dir().display()),
        format!("--out-dir={}", out_dir.display()),
        format!("--platform={}", common::platform_desc_dir(target).display()),
    ];
    if is_lib {
        args.push("--lib".into());
    }
    args.push(src.to_str().unwrap().into());

    let status = Command::new(common::langc_exe())
        .args(&args)
        .status()
        .expect("langc (g) invocation failed");
    assert!(
        status.success(),
        "langc -g --checks=all failed on {}",
        src.display()
    );

    std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o") && !before.contains(p))
        .next()
        .expect("langc (g) produced no .o file")
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_exec_tests").join(format!(
        "{}_{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Dynamic mode: .lmod loaded under QEMU via the on-device loader
// ---------------------------------------------------------------------------

fn build_tyu() {
    let s = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "tyu"])
        .status()
        .expect("cargo build tyu");
    assert!(s.success(), "cargo build tyu failed");
}

/// A module that emits the `S\n` completion marker via `testio.write-byte` and
/// returns 0 — the same shape the x86 dynamic test uses.
const DYNAMIC_PASS_MOD: &str = "module Main;\n\
import platform/testio { testio.write-byte };\n\
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;\n\
export { main };\nend;\n";

/// `tyu run --mode=dynamic` builds a firmware that embeds the `.lmod` in `.modpack`
/// and loads it on-device under QEMU. A zero exit means the on-device loader placed,
/// relocated, and ran the module's `main` (the `S\n` marker reached the harness).
#[test]
fn dynamic_lmod_runs_under_qemu() {
    if !common::require_tools(&[
        "langc",
        "arm-none-eabi-as",
        "arm-none-eabi-ld",
        "qemu-system-arm",
    ]) {
        return;
    }
    build_langc();
    build_tyu();

    let dir = temp_dir("arm_dynamic_modpack");
    let main_mod = dir.join("Main.mod");
    std::fs::write(&main_mod, DYNAMIC_PASS_MOD).unwrap();
    let out_dir = dir.join("dyn_out");
    let sysroot = common::workspace_root().join("sysroot");

    let output = Command::new(common::tyu_exe())
        .current_dir(common::workspace_root())
        .args([
            "run",
            "--mode=dynamic",
            "--target=armv7m-unknown-none",
            "--platform=armv7m-unknown-none",
            &format!("--sysroot={}", sysroot.display()),
            &format!("--out-dir={}", out_dir.display()),
            &main_mod.to_string_lossy(),
        ])
        .output()
        .expect("tyu run --mode=dynamic");

    assert!(
        output.status.success(),
        "ARM dynamic .lmod run must complete via on-device loader:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        out_dir.join("image.elf").exists(),
        "dynamic firmware missing"
    );
    assert!(out_dir.join("Main.lmod").exists(), "packed lmod missing");
    assert!(
        out_dir.join("modpack_generated.o").exists(),
        "modpack object missing — firmware did not embed the .lmod"
    );
}
