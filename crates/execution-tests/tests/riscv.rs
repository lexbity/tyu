//! RISC-V RV32 execution tests via `tyu test` driver and direct QEMU runs.

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
    if !common::require_tool_groups(&[
        &["langc"],
        common::RISCV_AS,
        common::RISCV_LD,
        &["qemu-system-riscv32"],
    ]) {
        return;
    }
    build_langc();

    let output = Command::new(common::tyu_exe())
        .args([
            "test",
            "--target=riscv32-unknown-none",
            &format!("--manifest={}", common::fixtures_manifest().display()),
        ])
        .output()
        .expect("tyu test");

    assert!(
        output.status.success(),
        "tyu test riscv32-unknown-none failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// D diagnostic record from __lang_trap_loc (header-only RISC-V agent)
// ---------------------------------------------------------------------------

#[test]
fn trap_emits_framed_d_record() {
    if !common::require_tool_groups(&[
        &["langc"],
        common::RISCV_AS,
        common::RISCV_LD,
        &["qemu-system-riscv32"],
    ]) {
        return;
    }
    build_langc();

    let dir = temp_dir("riscv_trap_d");
    let target = codegen_core::Target::RiscV32UnknownNone;

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

    let outcome = run_qemu_riscv(&image, std::time::Duration::from_secs(10));
    assert!(!outcome.timed_out, "RISC-V trap fixture must not hang");

    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();

    let d_records: Vec<_> = records
        .iter()
        .filter(|r| matches!(r, harness_core::Record::Diag(_)))
        .collect();
    assert!(
        !d_records.is_empty(),
        "RISC-V trap output must contain D record"
    );

    for rec in &d_records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag = diag_core::DiagRecord::parse(payload).expect("valid DiagRecord on RISC-V");
            assert_eq!(
                diag.trap_code, 21,
                "expected SUBTYPE_FAIL (21), got {}",
                diag.trap_code,
            );
            assert!(diag.valid, "RISC-V __lang_trap_loc must have valid=1");
            assert_eq!(diag.origin, diag_core::origin::IN_GUEST);
            assert_eq!(diag.version, diag_core::DIAG_RECORD_VERSION);
            assert_ne!(diag.word_hash, 0, "word_hash must not be zero");
            assert_eq!(
                diag.slot_count, 0,
                "RISC-V is header-only, slot_count must be 0"
            );
        }
    }
}

#[test]
fn trap_overflow_emits_trap_code_10() {
    if !common::require_tool_groups(&[
        &["langc"],
        common::RISCV_AS,
        common::RISCV_LD,
        &["qemu-system-riscv32"],
    ]) {
        return;
    }
    build_langc();

    let dir = temp_dir("riscv_trap_overflow");
    let target = codegen_core::Target::RiscV32UnknownNone;

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

    let outcome = run_qemu_riscv(&image, std::time::Duration::from_secs(10));
    assert!(!outcome.timed_out, "RISC-V stack overflow must not hang");

    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();

    for rec in &records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag =
                diag_core::DiagRecord::parse(payload).expect("valid DiagRecord on RISC-V overflow");
            assert_eq!(
                diag.trap_code, 10,
                "stack overflow must report trap_code=10, got {}",
                diag.trap_code,
            );
            assert!(!diag.valid, "overflow has no payload, valid must be 0");
            assert_eq!(diag.slot_count, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime symbol exports
// ---------------------------------------------------------------------------

#[test]
fn runtime_exports_required_symbols() {
    if !common::require_tool_groups(&[common::RISCV_AS, common::RISCV_NM]) {
        return;
    }

    let target = codegen_core::Target::RiscV32UnknownNone;
    let out_dir = common::temp_dir("riscv_runtime_symcheck");
    let runtime_objs = common::assemble_runtime(target, &out_dir);
    let nm_bin = common::first_available(common::RISCV_NM)
        .expect("no RISC-V nm available (checked RISCV_NM)");

    let mut all_symbols = String::new();
    for obj in &runtime_objs {
        let output = Command::new(&nm_bin)
            .arg("--defined-only")
            .arg(obj)
            .output()
            .unwrap_or_else(|_| panic!("{nm_bin} invocation failed"));
        assert!(output.status.success(), "nm failed on {:?}", obj);
        all_symbols.push_str(&String::from_utf8_lossy(&output.stdout));
    }

    // Use feature-derived expected symbol set (DEBT-3).
    let has_modload = std::path::Path::new("runtime/riscv32-unknown-none/modload.asm").exists();
    let expected = if has_modload {
        common::expected_symbols(codegen_core::FeatureSet::all())
    } else {
        common::expected_symbols(codegen_core::FeatureSet::empty())
    };
    for sym in &expected {
        assert!(
            all_symbols.contains(sym),
            "RISC-V runtime missing required symbol `{sym}`:\n{all_symbols}",
        );
    }

    // Verify trap labels are at DIFFERENT addresses (no aliasing).
    let extract_addr = |sym: &str| -> Option<u64> {
        for line in all_symbols.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[1] == "T" && parts[2] == sym {
                return u64::from_str_radix(parts[0], 16).ok();
            }
        }
        None
    };

    let trap_addr = extract_addr("__lang_trap");
    let loc_addr = extract_addr("__lang_trap_loc");
    let ovf_addr = extract_addr("__stack_overflow");
    assert!(
        trap_addr.is_some() && loc_addr.is_some() && ovf_addr.is_some(),
        "all three trap symbols must be defined"
    );
    let addrs = vec![trap_addr.unwrap(), loc_addr.unwrap(), ovf_addr.unwrap()];
    let mut sorted = addrs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        3,
        "all three trap symbols must be at distinct addresses (no aliasing)"
    );
}

// ---------------------------------------------------------------------------
// QEMU runner helper (RISC-V semihosting)
// ---------------------------------------------------------------------------

struct QemuOutcome {
    stdout: Vec<u8>,
    timed_out: bool,
}

fn run_qemu_riscv(image: &PathBuf, timeout: std::time::Duration) -> QemuOutcome {
    use std::io::Read;
    use std::time::Instant;

    let mut cmd = Command::new("qemu-system-riscv32");
    cmd.arg("-machine")
        .arg("virt")
        // Suppress the default OpenSBI firmware so the CPU resets directly into
        // our kernel at 0x80000000 (otherwise the two overlap and nothing runs).
        .arg("-bios")
        .arg("none")
        // Route semihosting output to a stdio chardev so it lands on stdout
        // (the default console sends it to QEMU's stderr, which we don't capture).
        .arg("-display")
        .arg("none")
        .arg("-serial")
        .arg("none")
        .arg("-monitor")
        .arg("none")
        .arg("-chardev")
        .arg("stdio,id=sh0")
        .arg("-semihosting-config")
        .arg("enable=on,target=native,chardev=sh0")
        .arg("-kernel")
        .arg(image);

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("qemu-system-riscv32 spawn failed");

    let mut stdout_pipe = child.stdout.take().unwrap();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let stdout = stdout_handle.join().unwrap_or_default();
                return QemuOutcome {
                    stdout,
                    timed_out: false,
                };
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stdout = stdout_handle.join().unwrap_or_default();
                    return QemuOutcome {
                        stdout,
                        timed_out: true,
                    };
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => {
                let stdout = stdout_handle.join().unwrap_or_default();
                return QemuOutcome {
                    stdout,
                    timed_out: false,
                };
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Compilation helpers
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
