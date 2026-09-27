//! §11.2 acceptance item 2 (PLAN-VERIFY-3): the consumption-side tamper
//! matrix fails closed. The kernel-checked/harvest-side tamper items (mutate
//! Gen ⇒ E6418, sorry ⇒ E6419, delete theorem ⇒ E6420) are the port gate's
//! negatives (`ci/port.sh` P7.1, `verification/ports/lean/tests/tamper/`);
//! this suite pins the *langc consumption* surface:
//!
//! 1. mutate a verdict's `statement_hash` ⇒ stale ⇒ open, check retained,
//!    counted (E6421) — never a silent reuse;
//! 2. feed an unrecognized certifier's `proof` label ⇒ downgraded to
//!    `assumed` with the original claim preserved in `claimed` (§Q6) — and
//!    such a record is NOT `proven`-admissible;
//! 3. a stale `(target, model_semantics)` identity on the file ⇒ stale ⇒
//!    open, check retained.
//!
//! Every tamper MUST fail closed to *more* checking: the build still
//! compiles (the checks are retained) and no wrong `proof` ever flows.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-tamper-matrix-{tag}-{}-{}",
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

/// The certified surface: pass-1 artifact + its exact statement hashes.
struct Certified {
    artifact: PathBuf,
    doc: String,
}

fn certified(dir: &Path) -> Certified {
    common::ensure_bins();
    fs::write(dir.join("Bank.mod"), MOD).unwrap();
    let p1 = dir.join("p1");
    fs::create_dir_all(&p1).unwrap();
    let s = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", p1.display()))
        .arg(dir.join("Bank.mod").to_str().unwrap())
        .status()
        .unwrap();
    assert!(s.success());
    let artifact = p1.join("Bank.obl.json");
    let set = verifier::codec::read_obl(&fs::read(&artifact).unwrap()).unwrap();
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
    let doc = format!(
        "{{\"schema\":\"tyu.verdicts/v2\",\"certifier\":{{\"class\":\"port\",\"name\":\"lean\",\"recognition\":\"tyu-port/lean/1\",\"tool\":{{\"name\":\"harvest\",\"version\":\"0.1.0\"}},\"toolchain\":\"lean4:4.27.0\"}},\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{}]}}",
        records.join(",")
    );
    Certified { artifact, doc }
}

/// Compile with the given verdicts document + `--bind-obl`; returns
/// (success, stderr, out-dir).
fn compile(dir: &Path, doc: &str, bind: Option<&Path>) -> (bool, String, PathBuf) {
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let vf = dir.join("v.json");
    fs::write(&vf, doc).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--write-obl")
        .arg("--checks=undischarged")
        .arg(format!("--verdicts={}", vf.display()));
    if let Some(b) = bind {
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
        .expect("echo must parse")
}

fn asm_traps(out_dir: &Path) -> usize {
    let asm = fs::read_to_string(out_dir.join("Bank.asm")).unwrap();
    asm.lines()
        .filter(|l| l.trim() == "jmp __lang_trap")
        .count()
}

/// Baseline: the untampered document binds and elides (the control for the
/// tamper matrix).
#[test]
fn control_untampered_binds_and_elides() {
    let dir = fresh_dir("control");
    let c = certified(&dir);
    let (ok, err, out_dir) = compile(&dir, &c.doc, Some(&c.artifact));
    assert!(ok, "control must build: {err}");
    let echo = read_echo(&out_dir);
    assert_eq!(echo.stale_verdicts, 0);
    assert_eq!(
        echo.verdicts
            .records
            .iter()
            .filter(|r| r.status == verifier::verdict::VerdictStatus::Discharged)
            .count(),
        6
    );
    assert_eq!(echo.emitted.subtype_range, 0, "controls elide");
}

/// Tamper item: mutate a verdict's `statement_hash` (a weakened/different
/// statement) ⇒ those sites resolve stale ⇒ open, check retained, counted
/// (E6421). Fail-closed to more checking — never a wrong `proof`.
#[test]
fn mutated_statement_hash_is_stale_open_retained() {
    let dir = fresh_dir("hash");
    let c = certified(&dir);
    // Tamper EVERY record's statement_hash (a weakened/different statement).
    let set = verifier::codec::read_obl(&fs::read(&c.artifact).unwrap()).unwrap();
    let records: Vec<String> = set
        .obligations
        .iter()
        .map(|o| {
            format!(
                "{{\"id\":{},\"id_hash\":{},\"status\":\"discharged\",\"trust\":\"proof\",\"method\":\"certificate\",\"statement_hash\":\"deadbeef{}\",\"proof\":{{\"kind\":\"certificate\",\"statement\":\"tyu.stmt/1.0\"}}}}",
                json(&o.id),
                json(&o.id_hash),
                "00".repeat(32)
            )
        })
        .collect();
    let doc = format!(
        "{{\"schema\":\"tyu.verdicts/v2\",\"certifier\":{{\"class\":\"port\",\"name\":\"lean\",\"recognition\":\"tyu-port/lean/1\",\"tool\":{{\"name\":\"harvest\",\"version\":\"0.1.0\"}},\"toolchain\":\"lean4:4.27.0\"}},\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"x86_64-unknown-linux-gnu\",\"model_semantics\":\"unmodeled\",\"verdicts\":[{}]}}",
        records.join(",")
    );
    let (ok, err, out_dir) = compile(&dir, &doc, Some(&c.artifact));
    assert!(
        ok,
        "mutated hashes must fail closed to more checking: {err}"
    );
    assert!(err.contains("E6421"), "stale diagnostic expected: {err}");
    let echo = read_echo(&out_dir);
    assert_eq!(echo.stale_verdicts, 6, "every mutated record counted stale");
    assert_eq!(
        echo.verdicts.records.len(),
        0,
        "no tampered record may close a site"
    );
    assert!(
        echo.emitted.subtype_range > 0,
        "checks retained at every stale site"
    );
    assert!(asm_traps(&out_dir) > 0, "traps retained in the object");
}

/// Tamper item: a wrong `(target, model_semantics)` identity on the file ⇒
/// every identity-mismatched record is stale ⇒ open, check retained (FR-5 —
/// a certificate for another build is worthless).
#[test]
fn wrong_identity_is_stale_open_retained() {
    let dir = fresh_dir("identity");
    let c = certified(&dir);
    // The artifacts' statements are tagged x86_64-unknown-linux-gnu; pretend
    // the certificate is for the armv7m target.
    let doc = c.doc.replacen(
        "\"target\":\"x86_64-unknown-linux-gnu\"",
        "\"target\":\"armv7m-unknown-none\"",
        1,
    );
    let (ok, err, out_dir) = compile(&dir, &doc, Some(&c.artifact));
    assert!(
        ok,
        "identity mismatch must fail closed to more checking: {err}"
    );
    let echo = read_echo(&out_dir);
    assert_eq!(echo.stale_verdicts, 6, "every record stale on identity");
    assert_eq!(echo.verdicts.records.len(), 0, "no record may close a site");
    assert!(echo.emitted.subtype_range > 0, "checks retained");
}

/// Tamper item: an unrecognized certifier's `proof` label is downgraded to
/// `assumed` with the original claim preserved in `claimed` (§Q6), and the
/// resulting `assumed` record never closes a `proven` site.
#[test]
fn unrecognized_certifier_downgrades_proof_to_assumed() {
    let dir = fresh_dir("certifier");
    let c = certified(&dir);
    let doc = c.doc.replacen("tyu-port/lean/1", "tyu-port/mystery/9", 1);
    // The codec-level downgrade guarantee (§Q6): reading the same document
    // through the input surface preserves the original claim in `claimed`.
    let v = verifier::verdict::read_verdicts(doc.as_bytes())
        .unwrap()
        .restrict_to_recognized();
    assert_eq!(v.records.len(), 6);
    assert_eq!(v.records[0].trust, verifier::verdict::Trust::Assumed);
    let claimed = v.records[0].claimed.as_ref().expect("claimed preserved");
    assert_eq!(claimed.trust.as_deref(), Some("proof"));
    assert_eq!(claimed.status.as_deref(), Some("discharged"));
    assert_eq!(
        claimed.method.as_deref(),
        Some("certificate"),
        "the claimed method is preserved verbatim"
    );
    // langc consumption: every downgraded record resolves Assumed (never
    // proof), and the build still compiles with checks retained.
    let (ok, err, out_dir) = compile(&dir, &doc, Some(&c.artifact));
    assert!(ok, "downgraded records must still build: {err}");
    let echo = read_echo(&out_dir);
    let assumed = echo
        .verdicts
        .records
        .iter()
        .filter(|r| r.trust == verifier::verdict::Trust::Assumed)
        .collect::<Vec<_>>();
    assert!(
        !assumed.is_empty(),
        "an unrecognized producer's proof labels must downgrade to assumed"
    );
    for r in assumed {
        assert_eq!(
            r.trust,
            verifier::verdict::Trust::Assumed,
            "never a proof from an unrecognized producer: {:?}",
            r
        );
    }
    // Under `proven`, the downgraded-assumed records are NOT admitted: the
    // site stays open (the in-tree fallback is also not admitted).
    let (ok, err, out_dir) = compile_proven_after(&dir);
    assert!(
        ok,
        "proven build with downgraded records still compiles: {err}"
    );
    let echo = read_echo(&out_dir);
    // The certificate-bound records stale or not-admitted; every site open.
    assert!(
        echo.verdicts.records.is_empty(),
        "no downgraded record may close a proven site"
    );
}

/// Recompile `Bank.mod` under `--verify-policy=proven` reusing the same
/// verdicts + bind artifact dirs.
fn compile_proven_after(dir: &Path) -> (bool, String, PathBuf) {
    let out_dir = dir.join("out-proven");
    fs::create_dir_all(&out_dir).unwrap();
    let vf = dir.join("v.json");
    let mut cmd = Command::new(langc_exe());
    cmd.arg("--emit=obj")
        .arg("--target=x86_64-unknown-linux-gnu")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--write-obl")
        .arg("--checks=undischarged")
        .arg("--verify-policy=proven")
        .arg(format!("--verdicts={}", vf.display()))
        .arg(format!(
            "--bind-obl={}",
            dir.join("p1").join("Bank.obl.json").display()
        ))
        .arg(dir.join("Bank.mod").to_str().unwrap());
    let out = cmd.output().expect("langc invocation");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out_dir,
    )
}
