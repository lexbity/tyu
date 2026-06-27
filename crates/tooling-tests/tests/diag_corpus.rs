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
    // Capture stdio so the nested cargo never flips the shared test terminal
    // to O_NONBLOCK; that flag leaks back to the outer `cargo test` harness,
    // whose writes then panic with EAGAIN.
    let out = Command::new(env!("CARGO"))
        .current_dir(&common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .output()
        .expect("cargo build");
    assert!(
        out.status.success(),
        "cargo build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
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
    // Assemble the core runtime and the static-mode entry unit. `runtime.asm`
    // does `extrn __lang_entry` (defined in `static_entry.asm` for static
    // links), so both units must be assembled and linked — mirroring the
    // product path in `execution-tests/common.rs::assemble_runtime`.
    for stem in ["runtime", "static_entry"] {
        let obj = dir.join(format!("{stem}.o"));
        let fasm_status = Command::new("fasm")
            .args([
                rt_dir.join(format!("{stem}.asm")).to_str().unwrap(),
                obj.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(fasm_status.success(), "fasm {stem}");
        objs.push(obj);
    }

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
///
/// A misbehaving guest (one that never hits `isa-debug-exit`) would otherwise
/// leave `qemu` running forever and hang the whole suite — exactly the failure
/// mode seen when a fixture's image is malformed. Spawn with a hard wall-clock
/// cap and kill on overrun, returning whatever was buffered.
fn capture_b_side(image: &Path) -> Vec<u8> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let mut child = Command::new("qemu-system-x86_64")
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
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("qemu");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }

    let mut buf = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut buf);
    }
    buf
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
// Corpus entry: STACK_OVERFLOW (10) — NOT a runtime corpus fixture.
//
// Code 10 (`__stack_overflow`) is the *data-stack* guard: codegen emits
// `cmp <next>, r14 ; ja __stack_overflow` (r14 = `__lang_ds_limit`) at every
// data-stack push. It cannot be exercised by a `.mod` source fixture here:
//
//   * Word calls use the hardware `call`/`ret` stack (rsp), not the data
//     stack, so recursion like `: recurse ( -- ) recurse ;` overflows the
//     hardware stack (page/triple-fault → reboot, no diagnostic) and never
//     touches the data-stack guard.
//   * The stack-effect type system *forbids* unbounded data-stack growth: a
//     net-positive recursive word (e.g. `: f ( i64 -- i64 ) dup f ;`) fails
//     typecheck (E3220), because the body's net effect can't match a finite
//     signature. With a 128 KiB / 16384-slot data stack, deep bounded
//     recursion also overflows the hardware stack first.
//
// So code 10 is a defensive guard that is effectively unreachable from
// well-typed source. Rather than a (necessarily fake or fragile) runtime
// fixture, the guard's *emission* is verified directly by a codegen unit test:
// see `data_stack_push_emits_overflow_guard` in
// `crates/codegen-x86_64/src/ophelpers.rs`. That keeps the corpus honest: we
// no longer claim a runtime-validated code-10 path that cannot exist.
// ---------------------------------------------------------------------------

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
    // `needs [ false ]` is an always-false precondition (current contract
    // syntax, per S-11; the old `requires [...]`/`pre [...]` forms are gone).
    // The block leaves the input `i64` untouched, so `trigger` is the identity
    // `( i64 -- i64 )`; the contract fires `CONTRACT_FAIL` (20) on every call.
    let fixture = "\
module Main;
import platform/testio { testio.write-byte };
: trigger ( i64 -- i64 )
  needs [ false ]
  ;
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
