//! The `proven` trust gate (PLAN-VERIFY-3 §Q12, P7.3): only `proof`-class
//! records and exact-method `checked` (`descriptor`/`stack-exact`) close a
//! site; interval-alone and assumed stay open (checks retained) until P14's
//! rederive or a developer certificate upgrades them.
//!
//! Driven at the langc consumption surface (hermetic — no Lean toolchain):
//! the two-pass certificate path uses `--bind-obl` (the pass-1 artifact the
//! proofs were certified against; the live lowering's word IR is
//! verdict-dependent, so the recompute would stale-eval otherwise — FR-5)
//! plus a harvest-style `tyu.verdicts/v2` file carrying `proof`/`certificate`
//! records whose `statement_hash` matches the artifact.
//!
//! The report-level E6410 enforcement of `proven` is tyu's job (covered by
//! `crates/tyu/tests/build_verify_integration.rs::proven_policy_fails_closed_
//! without_proof_class_discharges`); this suite pins the *langc-side* gate:
//! what discharges, what is forced open, and with which reason.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-policy-proven-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

const MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
: clamp ( i64 -- Percent )
  dup 100 > [ drop 100 ] [ ] if
  dup 0 < [ drop 0 ] [ ] if
  as Percent ;
: bounded_inc ( Percent -- Percent ) 1 + as Percent ;
: main ( -- i64 ) 50 as Percent bounded_inc as i64 ;
export { main };
end;
";

const EMPTY: &str = "{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[]}";

fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Compile `Bank.mod` through langc with the given verdicts doc (or `None`
/// for no file) and `--verify-policy=proven`; returns (success, stderr,
/// out-dir). `bind_obl` optionally arms the two-pass binding surface.
fn compile_proven(
    dir: &Path,
    verdicts: Option<&str>,
    bind_obl: Option<&Path>,
) -> (bool, String, PathBuf) {
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--write-obl")
        .arg("--checks=undischarged")
        .arg("--verify-policy=proven");
    match verdicts {
        Some(body) => {
            let vf = dir.join("v.json");
            fs::write(&vf, body).unwrap();
            cmd.arg(format!("--verdicts={}", vf.display()));
        }
        None => {
            let vf = dir.join("empty.json");
            fs::write(&vf, EMPTY).unwrap();
            cmd.arg(format!("--verdicts={}", vf.display()));
        }
    }
    if let Some(b) = bind_obl {
        cmd.arg(format!("--bind-obl={}", b.display()));
    }
    cmd.arg(dir.join("Bank.mod").to_str().unwrap());
    let out = cmd.output().expect("langc invocation");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out_dir,
    )
}

fn read_echo(out_dir: &Path) -> verifier::verdict::Echo {
    verifier::verdict::read_echo(&fs::read(out_dir.join("Bank.verdicts.inTree.json")).unwrap())
        .expect("echo must parse (echo-is-input invariant, §7.4)")
}

/// Pass-1: the canonical obligation artifact (`--emit=obligations`) — the
/// statement surface the Gen statements and developer proofs bind against.
fn pass1(dir: &Path) -> PathBuf {
    let p1 = dir.join("p1");
    fs::create_dir_all(&p1).unwrap();
    let s = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", p1.display()))
        .arg(dir.join("Bank.mod").to_str().unwrap())
        .status()
        .unwrap();
    assert!(s.success(), "pass-1 extraction failed");
    p1.join("Bank.obl.json")
}

/// A harvest-style verdicts document: recognized certifier
/// (`tyu-port/lean/1`), every obligation discharged `proof`/`certificate`
/// with the statement hash computed from `artifact` (the pass-1 surface).
fn harvest_doc(artifact: &Path) -> String {
    let set = verifier::codec::read_obl(&fs::read(artifact).unwrap()).unwrap();
    let records: Vec<String> = set
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
            format!(
                "{{\"id\":{},\"id_hash\":{},\"status\":\"discharged\",\"trust\":\"proof\",\"method\":\"certificate\",\"surface\":\"ir\",\"statement_hash\":{},\"authored\":\"developer\",\"proof\":{{\"kind\":\"certificate\",\"statement\":\"tyu.stmt/1.0\",\"theorem\":\"obl_{}\",\"kernel_check\":\"lean-kernel+lean4checker\",\"file\":\"proofs/Bank.lean\"}}}}",
                json(&o.id),
                json(&o.id_hash),
                json(&sh),
                o.id.replace("::", "_").replace('-', "_")
            )
        })
        .collect();
    format!(
        "{{\"schema\":\"tyu.verdicts/v2\",\"certifier\":{{\"class\":\"port\",\"name\":\"lean\",\"recognition\":\"tyu-port/lean/1\",\"tool\":{{\"name\":\"harvest\",\"version\":\"0.1.0\"}},\"toolchain\":\"lean4:4.27.0\"}},\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{}]}}",
        records.join(",")
    )
}

// ---------------------------------------------------------------------------
// Accept: certificates {proof} + exact-method {checked} discharge under proven
// ---------------------------------------------------------------------------

/// The two-pass certificate path: harvest-style `proof` verdicts bound against
/// the pass-1 artifact (`--bind-obl`) discharge every site under `proven` —
/// all checks elide, no staleness, and the echo round-trips (§7.4).
#[test]
fn proven_certificates_bind_against_pass1_and_elide() {
    common::ensure_bins();
    let dir = fresh_dir("cert");
    fs::write(dir.join("Bank.mod"), MOD).unwrap();
    let artifact = pass1(&dir);
    let doc = harvest_doc(&artifact);
    let (ok, err, out_dir) = compile_proven(&dir, Some(&doc), Some(&artifact));
    assert!(ok, "proven build with bound certificates must pass: {err}");
    assert!(
        !err.contains("E6421"),
        "no statement-stale with the pass-1 bind artifact: {err}"
    );
    let echo = read_echo(&out_dir);
    let discharged = echo
        .verdicts
        .records
        .iter()
        .filter(|r| r.status == verifier::verdict::VerdictStatus::Discharged)
        .count();
    assert_eq!(discharged, 6, "all six certificates discharge");
    for r in &echo.verdicts.records {
        assert_eq!(
            r.trust,
            verifier::verdict::Trust::Proof,
            "every record keeps its proof class: {:?}",
            r
        );
        assert_eq!(
            r.method,
            Some(verifier::verdict::Method::Certificate),
            "certificate method survives the round-trip"
        );
        assert!(
            r.statement_hash.is_some(),
            "certificate records carry the bound statement_hash (echo-is-input)"
        );
        assert!(
            r.proof.is_some(),
            "proof object REQUIRED iff trust=proof (§6.3) — the echo must emit it"
        );
    }
    assert_eq!(echo.stale_verdicts, 0, "zero staleness");
    assert_eq!(echo.emitted.subtype_range, 0, "all checks elide");
}

/// Without `--bind-obl` the live lowering's verdict-dependent word IR
/// stale-evals every certificate (E6421) — the FR-5 gate, fail-closed to
/// more checking (never a silent reuse).
#[test]
fn proven_without_bind_artifact_stale_evals_everything() {
    common::ensure_bins();
    let dir = fresh_dir("nostalehel");
    fs::write(dir.join("Bank.mod"), MOD).unwrap();
    let artifact = pass1(&dir);
    let doc = harvest_doc(&artifact);
    let (ok, err, out_dir) = compile_proven(&dir, Some(&doc), None);
    assert!(
        ok,
        "stale certificates must fail closed to MORE checking: {err}"
    );
    assert!(err.contains("E6421"), "stale signal expected: {err}");
    let echo = read_echo(&out_dir);
    assert!(echo.stale_verdicts >= 4, "certificates must stale-eval");
    assert!(
        echo.emitted.subtype_range > 0,
        "checks retained at every stale site"
    );
}

// ---------------------------------------------------------------------------
// Reject: interval-alone and assumed do not discharge under proven
// ---------------------------------------------------------------------------

/// No verdicts file: the in-tree interval discharges are forced open with the
/// `policy-proven: interval not admitted` reason (checks retained).
#[test]
fn proven_forces_interval_checked_open() {
    common::ensure_bins();
    let dir = fresh_dir("interval");
    fs::write(dir.join("Bank.mod"), MOD).unwrap();
    let (ok, err, out_dir) = compile_proven(&dir, None, None);
    assert!(
        ok,
        "forced-open sites keep the checks; the build still compiles: {err}"
    );
    let echo = read_echo(&out_dir);
    // The interval engine can close 3 sites (clamp:1, bounded_inc:2, main:0);
    // under `proven` ALL of them must resolve open with the not-admitted reason.
    let admitted = echo
        .open_reasons
        .iter()
        .filter(|o| o.reason.contains("policy-proven"))
        .count();
    assert!(
        admitted >= 3,
        "interval discharges must be forced open under proven: {:?}",
        echo.open_reasons
    );
    assert!(
        echo.emitted.subtype_range >= 4,
        "all checks retained: {:?}",
        echo.emitted
    );
}

/// An `assumed` verdict is not admitted under `proven` — the site resolves
/// open with the not-admitted reason (the assertion-free assumption never
/// discharges a proven build).
#[test]
fn proven_rejects_assumed_as_not_admitted() {
    common::ensure_bins();
    let dir = fresh_dir("assumed");
    fs::write(dir.join("Bank.mod"), MOD).unwrap();
    let artifact = pass1(&dir);
    let set = verifier::codec::read_obl(&fs::read(&artifact).unwrap()).unwrap();
    // Assumed records for the first three obligations.
    let records: Vec<String> = set
        .obligations
        .iter()
        .take(3)
        .map(|o| {
            format!(
                "{{\"id\":{},\"id_hash\":{},\"status\":\"assumed\",\"trust\":\"assumed\",\"justification\":\"manual review 2026-10-01, docs/reviews/0043\"}}",
                json(&o.id),
                json(&o.id_hash)
            )
        })
        .collect();
    let doc = format!(
        "{{\"schema\":\"tyu.verdicts/v2\",\"certifier\":null,\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{}]}}",
        records.join(",")
    );
    let (ok, err, out_dir) = compile_proven(&dir, Some(&doc), None);
    assert!(ok, "forced-open assumed sites keep the checks: {err}");
    let echo = read_echo(&out_dir);
    let assumed_rejected = echo
        .open_reasons
        .iter()
        .filter(|o| o.reason.contains("policy-proven"))
        .count();
    assert!(
        assumed_rejected >= 3,
        "assumed records are not admitted under proven: {:?}",
        echo.open_reasons
    );
    assert_eq!(
        echo.verdicts.records.len(),
        0,
        "no assumed record may close a site under proven"
    );
}
