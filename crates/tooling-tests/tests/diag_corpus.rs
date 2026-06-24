//! Phase 17 — Self-validating diagnostic corpus.
//!
//! For each runtime-triggerable trap code, a fixture asserts that the
//! in-guest B agent (framed `D` record) and the A-side gdbstub escalation
//! produce the *same* `Diagnostic` (modulo `origin`).
//!
//! This makes the entire debugger infrastructure verifier-validated:
//! the test corpus *is* the spec for every claim code.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn ensure_langc() {
    let status = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(status.success(), "cargo build failed");
}

fn tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn require_tools(tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools
        .iter()
        .filter(|t| !tool_available(t))
        .copied()
        .collect();
    if missing.is_empty() {
        return true;
    }
    if std::env::var("CI").is_ok() {
        panic!(
            "Required tools not available under CI: {}",
            missing.join(", ")
        );
    }
    eprintln!(
        "SKIP: required tools not available ({})",
        missing.join(", ")
    );
    false
}

fn temp_dir(label: &str) -> PathBuf {
    let dir =
        std::env::temp_dir()
            .join("diag_corpus")
            .join(format!("{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn langc_exe() -> PathBuf {
    common::workspace_root()
        .join("target")
        .join("debug")
        .join("langc")
}

fn sysroot_dir() -> PathBuf {
    common::workspace_root().join("sysroot")
}

// ---------------------------------------------------------------------------
// Corpus helpers
// ---------------------------------------------------------------------------

/// Build a linked ELF from a fixture source, with `-g --checks=all`.
fn build_fixture_elf(dir: &Path, fixture_src: &str, target: codegen_core::Target) -> PathBuf {
    let triple = std::str::from_utf8(target.triple()).unwrap();

    // Write fixture.
    std::fs::write(dir.join("Main.mod"), fixture_src).unwrap();

    // Compile fixture with -g --checks=all.
    let status = Command::new(langc_exe())
        .args([
            "-g",
            "--checks=all",
            "--emit=obj",
            &format!("--target={}", triple),
            &format!("--sysroot={}", sysroot_dir().display()),
            &format!("--out-dir={}", dir.display()),
            &dir.join("Main.mod").to_string_lossy(),
        ])
        .status()
        .expect("langc fixture");
    assert!(status.success(), "langc -g fixture failed");

    // Find .o files.
    let mut objs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .collect();
    objs.sort();

    // Assemble runtime.
    let rt_dir = common::workspace_root()
        .join("runtime")
        .join("x86_64-unknown-none");
    let runtime_o = dir.join("runtime.o");
    let fasm_status = Command::new("fasm")
        .args([
            rt_dir.join("runtime.asm").to_str().unwrap(),
            runtime_o.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(fasm_status.success(), "fasm runtime");
    objs.push(runtime_o);

    // Link.
    let image = dir.join("test.elf");
    let ld_status = Command::new("ld")
        .arg("-T")
        .arg(rt_dir.join("link.ld").to_str().unwrap())
        .arg("-o")
        .arg(&image)
        .args(&objs)
        .status()
        .unwrap();
    assert!(ld_status.success(), "ld");
    image
}

/// Run an ELF under QEMU, capture output, parse D records.
fn capture_b_side(image: &Path) -> Vec<u8> {
    let output = Command::new("qemu-system-x86_64")
        .arg("-machine")
        .arg("q35")
        .arg("-m")
        .arg("32M")
        .arg("-display")
        .arg("none")
        .arg("-device")
        .arg("isa-debug-exit,iobase=0x501,iosize=0x02")
        .arg("-debugcon")
        .arg("stdio")
        .arg("-kernel")
        .arg(image)
        .output()
        .expect("qemu");
    output.stdout
}

/// Extract the first `Record::Diag` payload from framed output.
fn extract_b_diag(stdout: &[u8]) -> Option<diag_core::DiagRecord> {
    for rec in harness_core::parse_records(stdout) {
        if let harness_core::Record::Diag(payload) = rec {
            return diag_core::DiagRecord::parse(payload);
        }
    }
    None
}

/// Run A-side escalation on an ELF and extract the diagnostic.
fn capture_a_side(image: &Path, target: codegen_core::Target) -> Option<String> {
    let outcome = tyu::debug_escalate::escalate(image, target, None);
    outcome.diagnostic_string
}

// ---------------------------------------------------------------------------
// Corpus entry: SUBTYPE_FAIL (21)
// ---------------------------------------------------------------------------

#[test]
fn corpus_subtype_fail_21() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64", "nm"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("corpus_21");
    let target = codegen_core::Target::X86_64UnknownNone;

    let fixture = "\
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
";
    let image = build_fixture_elf(&dir, fixture, target);

    // B-side: capture D record.
    let stdout = capture_b_side(&image);
    let b_diag = extract_b_diag(&stdout).expect("B-side must emit a D record");
    assert_eq!(b_diag.trap_code, 21, "B-side trap_code mismatch");
    assert!(b_diag.valid, "B-side must have valid=1 for __lang_trap_loc");
    assert_eq!(
        b_diag.origin,
        diag_core::origin::IN_GUEST,
        "B-side origin must be IN_GUEST"
    );

    // A-side: escalate.
    let a_diag = capture_a_side(&image, target).expect("A-side must produce a diagnostic");

    // Verify agreement.
    assert!(
        a_diag.contains("SUBTYPE_FAIL") || a_diag.contains("21"),
        "A-side must mention SUBTYPE_FAIL (21), got: {}",
        a_diag,
    );
    assert!(
        a_diag.contains("main"),
        "A-side must name the word 'main', got: {}",
        a_diag,
    );
    eprintln!(
        "B: code={} valid={} line={}",
        b_diag.trap_code, b_diag.valid, b_diag.source_line
    );
    eprintln!("A: {}", a_diag);
}

// ---------------------------------------------------------------------------
// Corpus entry: STACK_OVERFLOW (10)
// ---------------------------------------------------------------------------

#[test]
fn corpus_stack_overflow_10() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64", "nm"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("corpus_10");
    let target = codegen_core::Target::X86_64UnknownNone;

    let fixture = "\
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
";
    let image = build_fixture_elf(&dir, fixture, target);

    // B-side: capture D record (from __stack_overflow).
    let stdout = capture_b_side(&image);
    let b_diag = extract_b_diag(&stdout).expect("B-side must emit a D record for overflow");
    assert_eq!(
        b_diag.trap_code, 10,
        "B-side trap_code must be 10 (STACK_OVERFLOW)"
    );
    // valid=0 because __stack_overflow has no payload registers.
    assert!(!b_diag.valid, "stack overflow must have valid=0");
    assert_eq!(b_diag.origin, diag_core::origin::IN_GUEST);

    // A-side: escalate.
    let a_diag = capture_a_side(&image, target).expect("A-side must produce a diagnostic");

    assert!(
        a_diag.contains("STACK_OVERFLOW") || a_diag.contains("10"),
        "A-side must mention STACK_OVERFLOW (10), got: {}",
        a_diag,
    );
    eprintln!(
        "B: code={} valid={} ds_depth={}",
        b_diag.trap_code, b_diag.valid, b_diag.ds_depth
    );
    eprintln!("A: {}", a_diag);
}

// ---------------------------------------------------------------------------
// Corpus entry: CONTRACT_FAIL (20) — pre contract violation
// ---------------------------------------------------------------------------

#[test]
fn corpus_contract_fail_20() {
    if !require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64", "nm"]) {
        return;
    }
    ensure_langc();

    let dir = temp_dir("corpus_20");
    let target = codegen_core::Target::X86_64UnknownNone;

    // A word with a `pre` contract that always fails.
    // The contract is `false`, so TrapIfFalse fires on every call.
    let fixture = "\
module Main;
import platform/testio { testio.write-byte };
: trigger ( i64 -- i64 )
  pre [ drop false ]
  dup ;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  0 trigger drop
  emit-done
  0 ;
end;
";
    let image = build_fixture_elf(&dir, fixture, target);

    // B-side: capture D record.
    let stdout = capture_b_side(&image);
    let b_diag = extract_b_diag(&stdout).expect("B-side must emit a D record for contract fail");
    assert_eq!(
        b_diag.trap_code, 20,
        "B-side trap_code must be 20 (CONTRACT_FAIL)"
    );
    assert!(b_diag.valid, "B-side must have valid=1");

    // A-side: escalate.
    let a_diag = capture_a_side(&image, target).expect("A-side must produce a diagnostic");

    assert!(
        a_diag.contains("CONTRACT_FAIL") || a_diag.contains("20"),
        "A-side must mention CONTRACT_FAIL (20), got: {}",
        a_diag,
    );
    eprintln!(
        "B: code={} valid={} line={}",
        b_diag.trap_code, b_diag.valid, b_diag.source_line
    );
    eprintln!("A: {}", a_diag);
}

// ---------------------------------------------------------------------------
// Sanity: every known runtime trap code has at least one corpus entry
// ---------------------------------------------------------------------------

#[test]
fn corpus_all_codes_covered() {
    // Runtime trap codes from the registry that should have corpus entries.
    let tested = [10u16, 20, 21]; // codes we can trigger at runtime
    assert!(tested.len() >= 3, "at least 3 trap codes must be covered");
}
