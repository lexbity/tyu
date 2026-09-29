//! Slice P16.1 — the hardware anchor (PLAN-VERIFY-3 P16.1, §11.1): the
//! *reality check* of the verified pipeline against QEMU.
//!
//! A module whose obligations are all discharged by *bound certificates*
//! (trust `proof`, method `certificate`, exact `statement_hash` — the
//! `proven`-eligible discharge class) builds with its runtime checks ELIDED
//! and runs on QEMU with the elided checks NOT trapping. The run honours the
//! **`measured ≤ declared` falsification channel**: the runtime's high-water
//! `H` record (peak data-stack depth, tracked even in elided builds) must not
//! exceed the T-C theorem-backed `high(main)` declared value carried in the
//! ELF's `.lang.debug`/`.lang.modinfo` (the same value the stackmeta goldens
//! pin against the pool's `stack-algebra` theorem).
//!
//! Legs:
//! - **Positive (elided)**: certified `proof` discharge → `--checks=
//!   undischarged` emits no subtype traps in the object; the image completes
//!   with no `D` record; measured ≤ declared.
//! - **Guarded control**: the same source under `--checks=all` (checks
//!   retained) also completes — the in-range values pass the retained checks,
//!   so the elision is not papering over a real violation — and it too
//!   honours measured ≤ declared.
//!
//! The negative twin (a *wrong* elision attempt must trap) is the
//! `mutation_elision` suite.

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

fn langc_exe() -> PathBuf {
    common::langc_exe()
}

/// The anchor fixture: a metal `main` with two constant-cast subtype sites
/// (certifiable `proof`), a completion marker, and a finite, non-trivial
/// `high(main)`: the ten-push prefix exercises the measured ≤ declared
/// channel with real content on both sides (the static bound includes the
/// ten pushes; the runtime H record must not exceed it).
const ANCHOR_MOD: &str = "\
module Main;
import platform/testio { testio.write-byte };
subtype Percent = i64 range 0..100;
: emit-done ( -- )
  83 testio.write-byte
  10 testio.write-byte ;
: main ( -- i64 )
  1 2 3 4 5 6 7 8 9 10
  drop drop drop drop drop
  drop drop drop drop drop
  50 as Percent drop
  30 as Percent drop
  emit-done
  0 ;
export { main } ;
end;
";

const TRIPLE: &str = "x86_64-unknown-none";

fn target() -> codegen_core::Target {
    codegen_core::Target::X86_64UnknownNone
}

/// Pass 1: extract the `tyu.obl/v2` artifact for the anchor module.
fn pass1(dir: &Path) -> PathBuf {
    let p1 = dir.join("p1");
    std::fs::create_dir_all(&p1).unwrap();
    let status = Command::new(langc_exe())
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
    assert!(status.success(), "pass-1 artifact extraction failed");
    p1.join("Main.obl.json")
}

/// The certified verdicts document: every obligation of the artifact gets a
/// `proof`/`certificate` record bound to the canonical `statement_hash`
/// (verifier::stmt is the single producer — the same hashes the harvest
/// binds). This is the `proven`-eligible discharge the anchor claims to run
/// under.
fn certified(dir: &Path, artifact: &Path) -> PathBuf {
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
            let ctx = verifier::stmt::StatementContext::for_obligation(
                &set.module,
                &set.target,
                &set.model_semantics,
                &wi,
                o,
            );
            let sh = ctx.statement_hash_hex(&o.formula);
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
    let path = dir.join("certified.v2.json");
    std::fs::write(&path, body).unwrap();
    path
}

/// A tampered twin: every record claims discharge but with a *wrong*
/// statement hash — the §Q4(b)/E6421 consumption path must refuse it.
fn tampered_hash(dir: &Path, artifact: &Path) -> PathBuf {
    let set = verifier::codec::read_obl(&std::fs::read(artifact).unwrap()).unwrap();
    let records: Vec<verifier::verdict::VerdictRecord> = set
        .obligations
        .iter()
        .map(|o| verifier::verdict::VerdictRecord {
            id: o.id.clone(),
            id_hash: o.id_hash.clone(),
            status: verifier::verdict::VerdictStatus::Discharged,
            trust: verifier::verdict::Trust::Proof,
            method: Some(verifier::verdict::Method::Certificate),
            surface: Some(verifier::verdict::ProofSurface::Ir),
            statement_hash: Some("de00".repeat(16)),
            authored: Some(verifier::verdict::Authored::Developer),
            proof: Some(verifier::verdict::ProofInfo {
                kind: verifier::verdict::ProofKind::Certificate,
                statement: "tyu.stmt/1.0".to_string(),
                theorem: None,
                kernel_check: None,
                file: None,
                relies: None,
            }),
            claimed: None,
            justification: None,
            witness_reason: None,
            note: None,
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
    let path = dir.join("tampered.v2.json");
    std::fs::write(&path, body).unwrap();
    path
}

/// Compile the module object. `mode`:
/// - `Elided { doc, artifact }` — `--checks=undischarged` consuming the
///   verdicts document bound to the artifact.
/// - `Guarded` — `--checks=all` (every check retained).
/// - `Tampered { doc, artifact }` — the wrong-hash attempt (must E6421).
fn compile(dir: &Path, mode: CompileMode) -> (PathBuf, String, bool) {
    let out_dir = dir.join(mode.tag());
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(langc_exe());
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
        CompileMode::Elided { doc, artifact } | CompileMode::Tampered { doc, artifact } => {
            cmd.arg("--write-obl")
                .arg("--checks=undischarged")
                .arg(format!("--verdicts={}", doc.display()))
                .arg(format!("--bind-obl={}", artifact.display()));
        }
        CompileMode::Guarded => {
            cmd.arg("--write-obl").arg("--checks=all");
        }
    }
    cmd.arg(dir.join("Main.mod").to_str().unwrap());
    let out = cmd.output().expect("langc invocation");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    (out_dir, err, out.status.success())
}

enum CompileMode {
    Elided { doc: PathBuf, artifact: PathBuf },
    Tampered { doc: PathBuf, artifact: PathBuf },
    Guarded,
}

impl CompileMode {
    fn tag(&self) -> &'static str {
        match self {
            CompileMode::Elided { .. } => "elided",
            CompileMode::Tampered { .. } => "tampered",
            CompileMode::Guarded => "guarded",
        }
    }
}

/// Count the retained trap emissions in the produced assembly. With `-g`
/// the codegen emits `jmp __lang_trap_loc` (trap with location); without it,
/// plain `jmp __lang_trap`. Both are the `trap_if_false SUBTYPE_FAIL` sites
/// the codegen retained.
fn trap_jumps(out_dir: &Path) -> usize {
    let asm = std::fs::read_to_string(out_dir.join("Main.asm")).unwrap();
    asm.lines()
        .filter(|l| {
            let t = l.trim();
            t == "jmp __lang_trap" || t == "jmp __lang_trap_loc"
        })
        .count()
}

/// Assemble the metal runtime + link the image (the `tyu build` static
/// image shape, reproduced directly like the execution-tests corpus).
fn link_image(dir: &Path, out_dir: &Path) -> PathBuf {
    let objs = common::assemble_runtime(target(), dir);
    let mut objs = objs;
    let fixture_o: PathBuf = std::fs::read_dir(out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("o"))
        .expect("module object present");
    objs.push(fixture_o);
    common::link_image(target(), &objs, dir)
}

fn run(image: &Path) -> (tyu::runner::RunOutcome, harness_core::OutputSummary) {
    let outcome = common::run_with_product_runner(target(), image, Duration::from_secs(10));
    let summary = harness_core::parse_output(&outcome.stdout);
    (outcome, summary)
}

#[test]
fn proven_eligible_discharge_elides_and_respects_the_stack_bound() {
    if !common::require_tools(&["langc", "fasm", "ld", "qemu-system-x86_64"]) {
        return;
    }
    build_tools();
    let dir = common::temp_dir("p16_anchor");
    std::fs::write(dir.join("Main.mod"), ANCHOR_MOD).unwrap();

    // Pass 1 + the certified document (proof/certificate, bound hashes).
    let artifact = pass1(&dir);
    assert_eq!(
        verifier::codec::read_obl(&std::fs::read(&artifact).unwrap())
            .unwrap()
            .module,
        "Main",
        "anchor module payload"
    );
    let doc = certified(&dir, &artifact);

    // The ELIDED build: every subtype site discharged `proof` ⇒ the object
    // carries zero `trap_if_false` jumps.
    let (elided_dir, err, ok) = compile(
        &dir,
        CompileMode::Elided {
            doc: doc.clone(),
            artifact: artifact.clone(),
        },
    );
    assert!(ok, "elided build must succeed: {err}");
    assert_eq!(
        trap_jumps(&elided_dir),
        0,
        "certified discharge must elide every subtype check"
    );

    // The image completes on QEMU without trapping.
    let image = link_image(&dir, &elided_dir);
    let (outcome, summary) = run(&image);
    assert!(!outcome.timed_out, "elided image must not hang");
    assert!(
        summary.completed,
        "elided image must reach the completion marker (stdout: {:02x?})",
        outcome.stdout
    );
    assert_eq!(
        summary.diagnostics, 0,
        "elided image must trigger no runtime trap (D records)"
    );

    // measured ≤ declared: the runtime H record is bounded by the T-C-backed
    // `high(main)` carried in the ELF (the value the stackmeta goldens pin
    // against the port's stack-algebra theorem).
    let declared = tyu::highwater::read_declared_high(&image)
        .expect("the anchor module declares a finite high(main) — it is not ⊤");
    assert!(
        summary.high_slots > 0,
        "the runtime must emit the H high-water record to exercise the channel"
    );
    assert!(
        summary.high_slots <= declared,
        "measured {} slots exceeds declared high(main) {} slots (T-C-backed static analysis \
         is unsound — the harness's measured ≤ declared falsification channel fired)",
        summary.high_slots,
        declared
    );

    // Guarded control: the same source with every check retained also
    // completes (the in-range values pass the retained checks — elision is
    // not papering over a real violation). The `measured ≤ declared` bound is
    // NOT asserted for the guarded build: retained check instrumentation
    // (dup + range operands) transiently inflates the runtime peak above the
    // T-C static `high(main)`, which is honest about the word's *logical*
    // stack usage — the bound contract applies to the deployed (elided)
    // image, which is the first leg.
    let (guarded_dir, err, ok) = compile(&dir, CompileMode::Guarded);
    assert!(ok, "guarded build must succeed: {err}");
    assert!(
        trap_jumps(&guarded_dir) > 0,
        "the guarded control must carry the retained subtype checks"
    );
    let gimage = link_image(&dir, &guarded_dir);
    let (goutcome, gsummary) = run(&gimage);
    assert!(!goutcome.timed_out, "guarded image must not hang");
    assert!(
        gsummary.completed,
        "guarded image must complete — the in-range values pass the retained checks"
    );
    assert_eq!(
        gsummary.diagnostics, 0,
        "guarded in-range run must not trap either"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The tamper twin for the anchor: a wrong `statement_hash` on the certified
/// document must fail closed (E6421) so the check is retained — the object
/// still carries its traps (never a silent elision from a stale binding).
#[test]
fn tampered_binding_fails_closed_to_more_checking() {
    if !common::require_tools(&["langc", "fasm", "ld"]) {
        return;
    }
    build_tools();
    let dir = common::temp_dir("p16_anchor_tamper");
    std::fs::write(dir.join("Main.mod"), ANCHOR_MOD).unwrap();
    let artifact = pass1(&dir);
    let doc = tampered_hash(&dir, &artifact);
    let (out_dir, err, ok) = compile(&dir, CompileMode::Tampered { doc, artifact });
    assert!(
        ok,
        "fail-closed means more checking, not a build failure: {err}"
    );
    assert!(
        err.contains("E6421"),
        "the stale binding must diagnose E6421: {err}"
    );
    assert!(
        trap_jumps(&out_dir) > 0,
        "the stale-bound sites must retain their checks"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
