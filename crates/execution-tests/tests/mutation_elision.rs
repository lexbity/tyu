//! Slice P16.1 — the mutation-elision negative (PLAN-VERIFY-3 P16.1, §11.1):
//! a *wrong elision* must never reach the device — and when the pipeline is
//! tricked into believing one (a tampered verdict document), the build fails
//! closed to *more checking* and the retained runtime check is the last-line
//! detector: its trap fires on QEMU.
//!
//! Mechanism under test (the "wrong elision injected via a tampered verdict
//! set is caught by the runtime assertion"):
//!
//! - The module drives an input-derived cast site (`pass ( n -- Percent )`)
//!   with 150 — out of the `0..100` subtype range at runtime.
//! - **Leg A (control, retained):** `--checks=all` keeps the check; the QEMU
//!   run TRAPS with `SUBTYPE_FAIL` (21) — the check is load-bearing.
//! - **Leg B (mutation):** a tampered verdict document claims the site
//!   discharged (`proof`, but with a *fabricated* statement hash). The
//!   consumption refuses it (E6421 → stale → open → check retained — §Q4(b));
//!   the run STILL TRAPS. The attempted wrong elision cannot happen; the
//!   runtime assertion is the last line.
//! - **Leg C (safe elision):** an in-range constant site certified with the
//!   *correct* bound `statement_hash` elides and runs clean — elision itself
//!   is correct, only the binding gate makes it safe.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn build_tools() {
    let s = Command::new(env!("CARGO"))
        .current_dir(common::workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build langc");
    assert!(s.success(), "cargo build langc failed");
}

fn target() -> codegen_core::Target {
    codegen_core::Target::X86_64UnknownNone
}

const TRIPLE: &str = "x86_64-unknown-none";

/// The module with an input-derived cast site: the fixpoint interval engine
/// cannot see through the `drive` call (`let the value be ⊤ — opaque`), so
/// the `as Percent` site is NOT statically dischargeable; `drive` feeds it
/// `150` — out of range at runtime.
const UNSAFE_MOD: &str = "\
module Main;
import platform/testio { testio.write-byte };
subtype Percent = i64 range 0..100;
: drive ( -- i64 ) 150 ;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  drive as Percent drop
  emit-done
  0 ;
export { main } ;
end;
";

/// The safe twin: the same cast site fed a constant in range; when its site
/// is certified against the correct statement, the check is genuinely
/// unnecessary and elides cleanly.
const SAFE_MOD: &str = "\
module Main;
import platform/testio { testio.write-byte };
subtype Percent = i64 range 0..100;
: drive ( -- i64 ) 50 ;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  drive as Percent drop
  emit-done
  0 ;
export { main } ;
end;
";

fn pass1(dir: &Path, mod_name: &str) -> PathBuf {
    let p1 = dir.join("p1");
    std::fs::create_dir_all(&p1).unwrap();
    let status = Command::new(common::langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--target={TRIPLE}"))
        .arg(format!("--out-dir={}", p1.display()))
        .arg(format!("--sysroot={}", common::sysroot_dir().display()))
        .arg(format!(
            "--platform={}",
            common::platform_desc_dir(target()).display()
        ))
        .arg(dir.join("Main.mod").to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "pass-1 extraction failed");
    let _ = mod_name;
    p1.join("Main.obl.json")
}

/// A verdicts document for the artifact's obligations. `good_hash` selects
/// the real statement binding; `false` fabricates one (the mutation).
fn document(dir: &Path, artifact: &Path, good_hash: bool) -> PathBuf {
    let set = verifier::codec::read_obl(&std::fs::read(artifact).unwrap()).unwrap();
    let records: Vec<verifier::verdict::VerdictRecord> = set
        .obligations
        .iter()
        .map(|o| {
            let wi = set
                .facts
                .words
                .iter()
                .find(|w| w.name == o.site.word)
                .map(|w| verifier::stmt::sha256_hex16(w.ir.as_bytes()))
                .unwrap_or_default();
            let sh = if good_hash {
                let ctx = verifier::stmt::StatementContext::for_obligation(
                    &set.module,
                    &set.target,
                    &set.model_semantics,
                    &wi,
                    o,
                );
                ctx.statement_hash_hex(&o.formula)
            } else {
                "de00".repeat(16)
            };
            verifier::verdict::VerdictRecord {
                id: o.id.clone(),
                id_hash: o.id_hash.clone(),
                status: verifier::verdict::VerdictStatus::Discharged,
                trust: verifier::verdict::Trust::Proof,
                method: Some(verifier::verdict::Method::Certificate),
                surface: Some(verifier::verdict::ProofSurface::Ir),
                statement_hash: Some(sh),
                authored: Some(verifier::verdict::Authored::Developer),
                proof: Some(verifier::verdict::ProofInfo {
                    kind: verifier::verdict::ProofKind::Certificate,
                    statement: "tyu.stmt/1.0".to_string(),
                    theorem: Some(format!("obl_{}", o.id.replace("::", "_").replace('-', "_"))),
                    kernel_check: Some("lean-kernel+lean4checker".to_string()),
                    file: Some("proofs/Main.lean".to_string()),
                    relies: None,
                }),
                claimed: None,
                justification: None,
                witness_reason: None,
                note: None,
            }
        })
        .collect();
    let cert = verifier::verdict::Certifier {
        class: "port".to_string(),
        name: "lean".to_string(),
        recognition: "tyu-port/lean/1".to_string(),
        tool: verifier::verdict::ToolIdentity {
            name: "harvest".to_string(),
            version: "0.1.0".to_string(),
        },
        toolchain: "lean4:4.27.0+anchor".to_string(),
    };
    let body = verifier::verdict::encode_verdicts(
        "harvest",
        "0.1.0",
        Some(&cert),
        &set.target,
        &set.model_semantics,
        &set.concurrency,
        &records,
        0,
        &verifier::verdict::EmittedChecksData {
            subtype_range: 0,
            contract: 0,
            mmio_bounds: 0,
        },
    )
    .expect("verdicts encode");
    let path = dir.join(if good_hash {
        "bound.v2.json"
    } else {
        "mutated.v2.json"
    });
    std::fs::write(&path, body).unwrap();
    path
}

fn compile(dir: &Path, mode: CompileMode) -> (PathBuf, String, bool) {
    let out_dir = dir.join(mode.tag());
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(common::langc_exe());
    cmd.arg("--emit=obj")
        .arg("-g")
        .arg(format!("--target={TRIPLE}"))
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(format!("--sysroot={}", common::sysroot_dir().display()))
        .arg(format!(
            "--platform={}",
            common::platform_desc_dir(target()).display()
        ));
    match &mode {
        CompileMode::Retained => {
            cmd.arg("--write-obl").arg("--checks=all");
        }
        CompileMode::Bound { doc, artifact } | CompileMode::Mutated { doc, artifact } => {
            cmd.arg("--write-obl")
                .arg("--checks=undischarged")
                .arg(format!("--verdicts={}", doc.display()))
                .arg(format!("--bind-obl={}", artifact.display()));
        }
    }
    cmd.arg(dir.join("Main.mod").to_str().unwrap());
    let out = cmd.output().expect("langc invocation");
    (
        out_dir,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

enum CompileMode {
    Retained,
    Bound { doc: PathBuf, artifact: PathBuf },
    Mutated { doc: PathBuf, artifact: PathBuf },
}

impl CompileMode {
    fn tag(&self) -> &'static str {
        match self {
            CompileMode::Retained => "retained",
            CompileMode::Bound { .. } => "bound",
            CompileMode::Mutated { .. } => "mutated",
        }
    }
}

fn trap_jumps(out_dir: &Path) -> usize {
    let asm = std::fs::read_to_string(out_dir.join("Main.asm")).unwrap();
    asm.lines()
        .filter(|l| {
            let t = l.trim();
            t == "jmp __lang_trap" || t == "jmp __lang_trap_loc"
        })
        .count()
}

fn link_image(dir: &Path, out_dir: &Path) -> PathBuf {
    let mut objs = common::assemble_runtime(target(), dir);
    let fixture_o: PathBuf = std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .expect("module object present");
    objs.push(fixture_o);
    common::link_image(target(), &objs, dir)
}

/// Run the image and assert it TRAPS with `SUBTYPE_FAIL` (21) — the runtime
/// assertion is the last-line detector. Returns the trap code found.
fn assert_traps_subtype(image: &Path) -> u16 {
    let outcome = common::run_with_product_runner(target(), image, Duration::from_secs(10));
    assert!(!outcome.timed_out, "a trapped fixture must trap, not hang");
    let summary = harness_core::parse_output(&outcome.stdout);
    assert!(
        !summary.completed,
        "the trapped run must NOT reach the completion marker (stdout: {:02x?})",
        outcome.stdout
    );
    assert!(
        summary.diagnostics > 0,
        "the retained check must emit a D diagnostic record"
    );
    let records: Vec<harness_core::Record<'_>> =
        harness_core::parse_records(&outcome.stdout).collect();
    let mut trap_code = 0u16;
    for rec in &records {
        if let harness_core::Record::Diag(payload) = rec {
            let diag = diag_core::DiagRecord::parse(payload).expect("valid DiagRecord");
            trap_code = diag.trap_code;
            break;
        }
    }
    assert_eq!(
        trap_code, 21,
        "expected SUBTYPE_FAIL (21) — the last-line detector firing, got {trap_code}"
    );
    trap_code
}

/// Leg A — the control: with the check retained (no discharge claim), the
/// out-of-range runtime value traps.
#[test]
fn retained_check_is_load_bearing_and_traps() {
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_tools();
    let dir = common::temp_dir("p16_mut_retained");
    std::fs::write(dir.join("Main.mod"), UNSAFE_MOD).unwrap();
    let (out_dir, err, ok) = compile(&dir, CompileMode::Retained);
    assert!(ok, "retained build must succeed: {err}");
    assert!(
        trap_jumps(&out_dir) > 0,
        "the cast site check must be retained"
    );
    let image = link_image(&dir, &out_dir);
    assert_traps_subtype(&image);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Leg B — the mutation: a tampered verdict document *claims* the unsafe
/// site discharged with a fabricated statement hash. The consumption refuses
/// (E6421 → stale → open → check retained), and the runtime still traps:
/// the wrong elision was caught at the build gate AND the retained check
/// fired at the last line.
#[test]
fn mutated_elision_claim_is_refused_and_the_retained_check_traps() {
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_tools();
    let dir = common::temp_dir("p16_mut_mutated");
    std::fs::write(dir.join("Main.mod"), UNSAFE_MOD).unwrap();
    let artifact = pass1(&dir, "Main");
    // The tamper: `good_hash = false` fabricates the statement binding.
    let doc = document(&dir, &artifact, false);
    let (out_dir, err, ok) = compile(
        &dir,
        CompileMode::Mutated {
            doc,
            artifact: artifact.clone(),
        },
    );
    assert!(
        ok,
        "fail-closed means more checking, never a silent wrong elision: {err}"
    );
    assert!(
        err.contains("E6421"),
        "the fabricated binding must diagnose E6421: {err}"
    );
    assert!(
        trap_jumps(&out_dir) > 0,
        "the mutated-claim site must retain its check — the wrong elision did not ship"
    );
    let image = link_image(&dir, &out_dir);
    assert_traps_subtype(&image);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Leg C — the safe twin: a genuinely-safe site (in-range constant) certified
/// against the *correct* statement hash elides and runs clean on QEMU. This
/// is the elision-side witness that the earlier legs were not testing a
/// broken runtime: elision is correct; the binding gate makes it safe.
#[test]
fn correctly_bound_elision_runs_clean() {
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_tools();
    let dir = common::temp_dir("p16_mut_bound");
    std::fs::write(dir.join("Main.mod"), SAFE_MOD).unwrap();
    let artifact = pass1(&dir, "Main");
    let doc = document(&dir, &artifact, true);
    let (out_dir, err, ok) = compile(&dir, CompileMode::Bound { doc, artifact });
    assert!(ok, "the bound build must succeed: {err}");
    assert_eq!(
        trap_jumps(&out_dir),
        0,
        "the correctly-bound safe site elides its check"
    );
    let image = link_image(&dir, &out_dir);
    let outcome = common::run_with_product_runner(target(), &image, Duration::from_secs(10));
    assert!(!outcome.timed_out, "the elided image must not hang");
    let summary = harness_core::parse_output(&outcome.stdout);
    assert!(
        summary.completed,
        "the elided safe run must complete (stdout: {:02x?})",
        outcome.stdout
    );
    assert_eq!(
        summary.diagnostics, 0,
        "no trap — the safe elision fires no runtime assertion"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
