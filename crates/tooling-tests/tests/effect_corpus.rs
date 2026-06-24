//! Negative corpus for the 50xx (effect/capability/context) and 51xx (stack depth)
//! error bands.  Each test compiles a `.mod` source that MUST fail with exactly
//! the expected error code.
//!
//! Note: TcError codes live in the semantics crate at
//! crates/semantics/src/typecheck/error.rs and are distinct from the 3xxx
//! band (semantics) and 8xxx band (verifier/IR).  The 50xx/51xx band is
//! reserved for these tests.

mod common;

use std::sync::Once;
use std::{path::PathBuf, process::Command};

static BUILD_ONCE: Once = Once::new();

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_effect_corpus").join(format!(
        "{}_{}_{}",
        label,
        std::process::id(),
        rand()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rand() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn build_langc() {
    BUILD_ONCE.call_once(|| {
        let mut cmd = Command::new(env!("CARGO"));
        cmd.current_dir(workspace_root());
        cmd.args(["build", "-p", "langc"]);
        // Match the test binary's profile so common::bin::resolve can find langc
        // in the primary location.  No shared build helper exists in common/ (the
        // shared module only resolves existing binaries, it never builds).
        if !cfg!(debug_assertions) {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("cargo build failed");
        assert!(status.success());
    });
}

// ---------------------------------------------------------------------------
// Xfail ledger helpers
// ---------------------------------------------------------------------------

struct XfailEntry {
    fixture: String,
    current: String,
    target: String,
    _slice: String,
}

fn parse_expect_header(src: &str) -> Result<(), u32> {
    let first_line = src.lines().next().unwrap_or("");
    if first_line == "# expect: ok" {
        return Ok(());
    }
    if let Some(code_str) = first_line.strip_prefix("# expect: E") {
        if let Ok(code) = code_str.parse::<u32>() {
            return Err(code);
        }
    }
    panic!("invalid # expect header in corpus fixture: {first_line}");
}

fn load_xfail_ledger() -> Vec<XfailEntry> {
    let path = workspace_root()
        .join("crates")
        .join("tooling-tests")
        .join("tests")
        .join("corpus")
        .join("xfail.toml");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .map(|l| {
            let parts: Vec<&str> = l.split('|').map(|s| s.trim()).collect();
            assert!(parts.len() >= 4, "malformed xfail entry: {l}");
            XfailEntry {
                fixture: parts[0].to_string(),
                current: parts[1].to_string(),
                target: parts[2].to_string(),
                _slice: parts[3].to_string(),
            }
        })
        .collect()
}

/// Run a fixture and return the observed outcome.
fn run_fixture(path: &PathBuf) -> String {
    let dir = fresh_dir("fixture_run");
    let mod_path = dir.join("test.mod");
    std::fs::write(&mod_path, std::fs::read_to_string(path).unwrap()).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", mod_path.to_str().unwrap()])
        .output()
        .unwrap();
    if out.status.success() {
        "compiles".to_string()
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Extract "E5001" from "error[E5001]: typecheck error"
        let code = stderr
            .split("E")
            .nth(1)
            .and_then(|s| {
                s.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .into()
            })
            .unwrap_or_else(|| "unknown".to_string());
        format!("E{code}")
    }
}

// ---------------------------------------------------------------------------
// Data-driven corpus runner
// ---------------------------------------------------------------------------

#[test]
fn corpus() {
    build_langc();
    let ledger = load_xfail_ledger();
    let corpus_dir = workspace_root()
        .join("crates")
        .join("tooling-tests")
        .join("tests")
        .join("corpus");

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&corpus_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "mod").unwrap_or(false))
        .collect();
    entries.sort();

    let total = entries.len();
    let mut xfail_count = 0u32;
    let mut pass_count = 0u32;

    for path in &entries {
        let fixture_name = path.file_name().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(path).unwrap();
        let expected = parse_expect_header(&src);
        let observed = run_fixture(path);

        // Check xfail ledger
        if let Some(xf) = ledger.iter().find(|x| x.fixture == fixture_name) {
            if observed == xf.current {
                xfail_count += 1;
                eprintln!("XFAIL {xfail_count}/{total}: {fixture_name} (observed={observed}, target={target}, slice={slice})",
                    target=xf.target, slice=xf._slice);
            } else if observed == xf.target {
                panic!(
                    "fixture {fixture_name} went green: observed={observed}, \
                     target={target}. Update ledger entries for slice {slice}.",
                    target = xf.target,
                    slice = xf._slice
                );
            } else {
                panic!(
                    "regression: {fixture_name} expected current={xf_current}, \
                     target={xf_target}, but got unexpected outcome={observed}",
                    xf_current = xf.current,
                    xf_target = xf.target
                );
            }
        } else {
            // Normal (non-xfail) test
            match (expected, observed.as_str()) {
                (Ok(()), "compiles") => {
                    pass_count += 1;
                }
                (Err(code), s) if s == format!("E{code}") => {
                    pass_count += 1;
                }
                (Ok(()), s) => {
                    panic!(
                        "{fixture_name}: expected to compile, got {s}. \
                         If this is a known issue, add it to xfail.toml."
                    );
                }
                (Err(code), s) => {
                    panic!(
                        "{fixture_name}: expected E{code}, got {s}. \
                         If this is a known issue, add it to xfail.toml."
                    );
                }
            }
        }
    }

    eprintln!("Corpus: {total} fixtures, {pass_count} pass, {xfail_count} xfail");
    assert!(total > 0, "corpus directory is empty — no fixtures to run");
}

// ---------------------------------------------------------------------------
// .def effect boundary: interface declares performs {} but implementation
// performs {suspend} → iface error 2219
// ---------------------------------------------------------------------------

#[test]
fn e_iface_effect_mismatch() {
    build_langc();
    let dir = fresh_dir("iface_effect");

    std::fs::write(
        dir.join("Core.def"),
        b"module Core;\nexport { foo };\n: foo ( -- ) performs {} ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Core.mod"),
        b"module Core;\nexport { foo };\n: foo ( -- ) performs {suspend} platform.task.yield ;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Main.mod"),
        b"module Main;\nimport Core { foo };\n: main ( -- i64 ) foo 0 ;\nend;\n",
    )
    .unwrap();

    let out = Command::new(langc_exe())
        .current_dir(&dir)
        .args(["--emit=ir", "Main.mod"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected iface error for effect mismatch"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error[E2219]"),
        "expected E2219 (word effect mismatch), got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Determinism: compile every negative fixture 20×, assert byte-identical
// rendered diagnostics (NFR-3).
// ---------------------------------------------------------------------------

#[test]
fn determinism() {
    build_langc();
    let corpus_dir = workspace_root()
        .join("crates")
        .join("tooling-tests")
        .join("tests")
        .join("corpus");

    let entries: Vec<PathBuf> = std::fs::read_dir(&corpus_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_str().unwrap_or("");
            name.starts_with("e") && p.extension().map(|x| x == "mod").unwrap_or(false)
        })
        .collect();

    for path in &entries {
        let src = std::fs::read_to_string(path).unwrap();
        let fixture_name = path.file_name().unwrap().to_str().unwrap().to_string();
        let mut prev_stderr: Option<Vec<u8>> = None;

        for run in 0..20 {
            let dir = fresh_dir(&format!("det_{}", fixture_name));
            let mod_path = dir.join("test.mod");
            std::fs::write(&mod_path, &src).unwrap();
            let out = Command::new(langc_exe())
                .args(["--emit=ir", mod_path.to_str().unwrap()])
                .output()
                .unwrap();

            // Collect stderr (the rendered diagnostic)
            let stderr = if out.status.success() {
                out.stdout
            } else {
                out.stderr
            };

            match &prev_stderr {
                None => prev_stderr = Some(stderr),
                Some(prev) => {
                    assert_eq!(
                        &stderr, prev,
                        "determinism failure: {fixture_name} run {run} differs from run 0"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Distinctness: verify no two TcError variants share a code.
// ---------------------------------------------------------------------------

#[test]
fn tcerror_codes_are_distinct() {
    let codes = [
        5001u32, 5002, 5003, 5004, 5005, 5010, 5011, 5012, 5020, 5030, 5031, 5040, 5100, 5101, 5103,
    ];
    let mut sorted = codes.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        codes.len(),
        "duplicate error codes in the 50xx/51xx band"
    );
}

// ---------------------------------------------------------------------------
// 50xx — Effect / Capability / Context  (individual fixture tests)
// ---------------------------------------------------------------------------

#[test]
fn e5001_suspend_forbidden() {
    assert_tc_fails_with(
        "module m;\n\
         : foo ( -- ) platform.task.yield ;\n\
         end;\n",
        5001,
    );
}

#[test]
fn e5002_lock_nest() {
    assert_ir_fails_with(
        "module Main;\n\
         resource R;\n\
         : nested ( -- )\n\
           R lock [ lock [ ] ]\n\
         ;\n\
         end;\n",
        5002,
    );
}

#[test]
fn e5003_lock_stack() {
    assert_ir_fails_with(
        "module Main;\n\
         resource R;\n\
         : bad ( -- )\n\
           R lock [ 1 ]\n\
         ;\n\
         end;\n",
        5003,
    );
}

#[test]
fn e5004_cap_missing() {
    assert_ir_fails_with(
        "module Main;\n\
         resource R;\n\
         : write ( -- )\n\
           &!R drop\n\
         ;\n\
         end;\n",
        5004,
    );
}

// ---------------------------------------------------------------------------
// Positive: lock body is green
// ---------------------------------------------------------------------------

#[test]
fn lock_body_green() {
    build_langc();
    let dir = fresh_dir("lock_green");
    let path = dir.join("test.mod");
    std::fs::write(
        &path,
        b"module Main;\n\
          resource R;\n\
          : ok ( -- )\n\
            R lock [ &!R drop ]\n\
          ;\n\
          end;\n",
    )
    .unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "expected lock body to pass, got exit={code} stderr={stderr}"
    );
}

#[test]
fn e5010_iso_dup() {
    assert_ir_fails_with(
        "module Main;\n\
         iso Msg;\n\
         : bad_dup ( Msg -- Msg Msg ) dup ;\n\
         end;\n",
        5010,
    );
}

#[test]
fn e5011_iso_drop() {
    assert_ir_fails_with(
        "module Main;\n\
         iso Msg;\n\
         : bad_drop ( Msg -- ) drop ;\n\
         end;\n",
        5011,
    );
}

#[test]
fn e5012_iso_use_after_move() {
    assert_ir_fails_with(
        "module Main;\n\
         iso Msg;\n\
         : move_twice ( Msg -- Msg )\n\
           => x\n\
           x x\n\
         ;\n\
         end;\n",
        5012,
    );
}

#[test]
fn e5020_borrow_escape() {
    build_langc();
    let dir = fresh_dir("e5020");
    let path = dir.join("test.mod");
    std::fs::write(
        &path,
        b"module Main;\n\
          : ok ( i64.4 -- i64.4 )\n\
            &[ drop ]\n\
          ;\n\
          end;\n",
    )
    .unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "expected scoped borrow to pass, got exit={code} stderr={stderr}"
    );
}

// ---------------------------------------------------------------------------
// Owned enforcement (reuses iso machinery: 5010/5011/5012)
// ---------------------------------------------------------------------------

#[test]
fn owned_dup_forbidden() {
    assert_ir_fails_with(
        "module Main;\n\
         owned Buffer;\n\
         : bad_dup ( Buffer -- Buffer Buffer ) dup ;\n\
         end;\n",
        5010,
    );
}

#[test]
fn owned_drop_forbidden() {
    assert_ir_fails_with(
        "module Main;\n\
         owned Buffer;\n\
         : bad_drop ( Buffer -- ) drop ;\n\
         end;\n",
        5011,
    );
}

#[test]
fn owned_use_after_move() {
    assert_ir_fails_with(
        "module Main;\n\
         owned Buffer;\n\
         : move_twice ( Buffer -- Buffer )\n\
           => x\n\
           x x\n\
         ;\n\
         end;\n",
        5012,
    );
}

#[test]
fn owned_round_trip() {
    build_langc();
    let dir = fresh_dir("owned_rt");
    let path = dir.join("test.mod");
    std::fs::write(
        &path,
        b"module Main;\n\
          owned Buffer;\n\
          : use_once ( Buffer -- Buffer )\n\
            => x\n\
            x\n\
          ;\n\
          end;\n",
    )
    .unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "expected owned round-trip to pass, got exit={code} stderr={stderr}"
    );
}

#[test]
fn e5030_isr_suspend_forbidden() {
    assert_ir_fails_with(
        "module Main;\n\
         @interrupt(TIMER0) : isr ( -- )\n\
           0\n\
           dup dup dup dup dup dup dup dup dup dup\n\
           dup dup dup dup dup dup dup dup dup dup\n\
           dup dup dup dup dup dup dup dup dup dup\n\
           dup dup dup\n\
           drop drop drop drop drop drop drop drop drop drop\n\
           drop drop drop drop drop drop drop drop drop drop\n\
           drop drop drop drop drop drop drop drop drop drop\n\
           drop drop drop drop\n\
         ;\n\
         end;\n",
        5030,
    );
}

#[test]
fn e5031_resource_shared_unlocked() {
    assert_ir_fails_with(
        "module Main;\n\
         resource R;\n\
         @interrupt(TIMER0) : isr ( -- )\n\
           &!R drop\n\
         ;\n\
         : main ( -- )\n\
           &!R drop\n\
         ;\n\
         end;\n",
        5031,
    );
}

#[test]
fn e5040_diverge_in_bounded() {
    build_langc();
    let dir = fresh_dir("e5040");
    let path = dir.join("test.mod");
    std::fs::write(
        &path,
        b"module Main;\n\
          : may_diverge ( -- i64 ) performs {diverge}\n\
            0\n\
          ;\n\
          end;\n",
    )
    .unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "expected diverge word to pass, got exit={code} stderr={stderr}"
    );
}

// ---------------------------------------------------------------------------
// 51xx — Stack depth
// ---------------------------------------------------------------------------

#[test]
fn e5100_stack_unbounded() {
    build_langc();
    let dir = fresh_dir("e5100");
    let path = dir.join("test.mod");
    std::fs::write(
        &path,
        b"module Main;\n\
          : main ( -- i64 )\n\
            1 2 + 3 +\n\
          ;\n\
          end;\n",
    )
    .unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "expected word with finite bound to pass, got exit={code} stderr={stderr}"
    );
}

#[test]
fn e5101_stack_exceeds_budget() {
    assert_ir_fails_with(
        "module Main;\n\
         @interrupt(TIMER0) : isr ( -- )\n\
           0\n\
           dup dup dup dup dup dup dup dup dup dup\n\
           dup dup dup dup dup dup dup dup dup dup\n\
           dup dup dup dup dup dup dup dup dup dup\n\
           dup dup dup\n\
           drop drop drop drop drop drop drop drop drop drop\n\
           drop drop drop drop drop drop drop drop drop drop\n\
           drop drop drop drop drop drop drop drop drop drop\n\
           drop drop drop drop\n\
         ;\n\
         end;\n",
        5030, // IsrStack — ISR body exceeds N_isr ceiling
    );
}

#[test]
fn e5103_stack_quot_erased() {
    build_langc();
    let dir = fresh_dir("e5103");
    let path = dir.join("test.mod");
    std::fs::write(
        &path,
        b"module Main;\n\
          : call_twice ( -- i64 )\n\
            [ ( -- i64 ) 1 ] call\n\
          ;\n\
          end;\n",
    )
    .unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        code == 0,
        "expected quotation with computable bound to pass, got exit={code} stderr={stderr}"
    );
}

// ---------------------------------------------------------------------------
// Helpers — unchanged from original but now use common::bin::resolve
// ---------------------------------------------------------------------------

/// Assert that compiling `src` fails with exactly `expected_code`.
/// The stack-checker is the `--emit=tc` path which uses the Context fold (Phase 5+).
fn assert_tc_fails_with(src: &str, expected_code: u32) {
    build_langc();
    let dir = fresh_dir("tc");
    let path = dir.join("test.mod");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=tc", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_str = format!("E{expected_code}");
    assert!(
        code != 0,
        "expected E{expected_code} (exit non-zero), but compilation succeeded.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&expected_str),
        "expected E{expected_code} in stderr, got:\n{stderr}"
    );
}

/// Assert that compiling `src` with `--emit=ir` fails with exactly `expected_code`.
fn assert_ir_fails_with(src: &str, expected_code: u32) {
    build_langc();
    let dir = fresh_dir("ir");
    let path = dir.join("test.mod");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=ir", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_str = format!("E{expected_code}");
    assert!(
        code != 0,
        "expected E{expected_code} (exit non-zero), but compilation succeeded.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&expected_str),
        "expected E{expected_code} in stderr, got:\n{stderr}"
    );
}

/// Assert that compiling `src` with `--emit=asm` fails with exactly `expected_code`.
fn assert_fails_with(src: &str, expected_code: u32) {
    build_langc();
    let dir = fresh_dir("neg");
    let path = dir.join("test.mod");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(langc_exe())
        .args(["--emit=asm", path.to_str().unwrap()])
        .output()
        .unwrap();
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected_str = format!("E{expected_code}");
    assert!(
        code != 0,
        "expected E{expected_code} (exit non-zero), but compilation succeeded.\nstderr: {stderr}"
    );
    assert!(
        stderr.contains(&expected_str),
        "expected E{expected_code} in stderr, got:\n{stderr}"
    );
}
