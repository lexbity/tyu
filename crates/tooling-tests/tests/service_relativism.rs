//! The concurrency-service relativism gate, end to end (PLAN-VERIFY-3 P15.1,
//! §Q14 / §Q15).
//!
//! A bundle's `[model] concurrency` declaration is the statement relativism's
//! concurrency dimension: a word that USES platform services renders its
//! statements against the abstract-atomic services model (`Tyu.Services`, the
//! §Q14 atomics) ONLY when the producing bundle declared
//! `concurrency = "abstract-atomic"`; otherwise the affected obligations fail
//! closed open with the `service-unmodeled` witness. This test drives the
//! REAL port machinery (`gen` renderer, harvest, langc) over the
//! `verification/ports/lean/tests/conc-fixture/` fixture (`module Conc`,
//! model `tyu.model/linux-x86_64-hosted/1`; `roundtrip-pct` rounds a
//! constant payload through a make-local channel and narrows the received
//! value to `Percent` [0, 100]):
//!
//!   1. **Relativism render**: under `abstract-atomic` the renderer emits the
//!      static channel-trace statement
//!      `Tyu.Services.traceInRange [make 0, send 0 42, recv 0] 0 100` and the
//!      gen.json header carries the concurrency declaration; under
//!      `unmodeled` the SAME artifact's obligations are omitted with
//!      `service-unmodeled` (fail closed — never a silent re-binding).
//!   2. **The template harvests a `proof`**: the temp lake package proves the
//!      FIFO round-trip statement (`ConcFix.lean` composes the §Q14
//!      `trace_send_recv_output` law with the subtype bound); harvest binds
//!      `trust: proof`, `method: certificate`, the recomputed statement hash.
//!      The opaque `$top` cast obligation (`::0`) stays `open` with
//!      `opaque-site` — the honest refusal (the renderer's classification
//!      for an obligation with no oel root, Render.lean `serviceClassify`).
//!   3. **FR-5 consumption + concurrency staleness**: langc consuming the
//!      verdicts with `--concurrency=abstract-atomic` discharges the proven
//!      site; consuming the SAME verdicts under `--concurrency=unmodeled` is
//!      E6421-stale (the file's concurrency declaration differs from the
//!      build's) — fail-closed to open, check retained (never a wrong
//!      discharge).
//!   4. **Service conformance (P15.2)**: the hosted bundle's
//!      `evidence/vectors.json` (`tyu.svcvec/1`) scripted channel programs
//!      replay identically in the Rust mirror and the port's
//!      `conformance --level services` — the FIFO/atomicity assertion,
//!      zero divergence (R9 detection).
//!
//! The test is tier-A style: it needs a Lean toolchain. Without one it SKIPS
//! with a message; `TYU_SERVICE_E2E=1` forces the run (and fails loudly when
//! the toolchain is absent). `ci/port.sh` runs it unconditionally (the port
//! gate always has the toolchain).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FIXTURE: &str = "verification/ports/lean/tests/conc-fixture";
const PORT: &str = "verification/ports/lean";
const OBL_ID: &str = "Conc::roundtrip-pct::subtype-range::1";
const OPAQUE_ID: &str = "Conc::roundtrip-pct::subtype-range::0";
const MODEL: &str = "tyu.model/linux-x86_64-hosted/1";

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn toolchain_present() -> bool {
    let lean = Command::new("lean").arg("--version").output();
    let lake = Command::new("lake").arg("--version").output();
    match (lean, lake) {
        (Ok(l), Ok(k)) => l.status.success() && k.status.success(),
        _ => false,
    }
}

fn fresh_dir(label: &str) -> PathBuf {
    common::fresh_dir(&format!("service_{label}"))
}

/// Render an artifact with the port's `gen` renderer into `<dir>/Gen`.
fn render_fixture(root: &Path, dir: &Path, artifact_name: &str) {
    let gen_exe = root.join(PORT).join(".lake/build/bin/gen");
    if !gen_exe.is_file() {
        Command::new("lake")
            .current_dir(root.join(PORT))
            .args(["build", "gen"])
            .status()
            .expect("lake build gen");
    }
    let status = Command::new(&gen_exe)
        .current_dir(dir)
        .arg("--render")
        .arg("--obl")
        .arg(dir.join(format!("{artifact_name}.obl.json")))
        .arg("--out")
        .arg(dir.join("Gen"))
        .status()
        .unwrap();
    assert!(status.success(), "gen --render failed");
}

/// Assemble the temp lake package: the port library + the generated Gen +
/// the developer proof.
fn assemble_package(root: &Path, dir: &Path, artifact_name: &str, proof: &str) {
    let pin = fs::read(root.join(PORT).join("lean-toolchain")).unwrap();
    fs::write(dir.join("lean-toolchain"), pin).unwrap();
    fs::create_dir_all(dir.join("Gen")).unwrap();
    render_fixture(root, dir, artifact_name);
    let lakefile = format!(
        "name = \"tyu-service-e2e\"\nversion = \"0.1.0\"\n\n[[lean_lib]]\nname = \"Tyu\"\nsrcDir = \"{}\"\nroots = [\"Tyu\"]\n\n[[lean_lib]]\nname = \"Gen\"\nsrcDir = \".\"\nroots = [\"Gen\"]\n\n[[lean_lib]]\nname = \"ConcFix\"\nsrcDir = \".\"\nroots = [\"ConcFix\"]\n",
        root.join(PORT).display()
    );
    fs::write(dir.join("lakefile.toml"), lakefile).unwrap();
    fs::write(dir.join("ConcFix.lean"), proof).unwrap();
    fs::write(
        dir.join("hv.lean"),
        "import ConcFix\nimport Tyu.Verdicts.Harvest\n\n#eval! Tyu.Verdicts.Harvest.run\n",
    )
    .unwrap();
}

fn run_harvest(dir: &Path) -> String {
    let build = Command::new("lake")
        .current_dir(dir)
        .args(["build", "ConcFix", "Tyu.Verdicts.Harvest"])
        .status()
        .unwrap();
    assert!(build.success(), "lake build of the fixture package failed");
    let out = dir.join("out.v2.json");
    let status = Command::new("lake")
        .current_dir(dir)
        .env("TYU_HARVEST_GEN_DIR", dir.join("Gen"))
        .env("TYU_HARVEST_OBL", dir.join("Conc.obl.json"))
        .env("TYU_HARVEST_OUT", &out)
        .args(["env", "lean", "hv.lean"])
        .status()
        .unwrap();
    assert!(status.success(), "harvest exited nonzero");
    fs::read_to_string(&out).unwrap()
}

/// Compile the real source under the hosted bundle; returns
/// `(success, stderr)`.
fn compile_conc(root: &Path, dir: &Path, concurrency: &str, proven: bool) -> (bool, String) {
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut args: Vec<String> = vec![
        "--emit=obj".into(),
        "--target=x86_64-unknown-linux-gnu".into(),
        format!("--out-dir={}", out_dir.display()),
        "--write-obl".into(),
        "--checks=undischarged".into(),
        format!("--platform={}", root.join("runtime").display()),
        format!("--sysroot={}", root.join("sysroot").display()),
        "--model-semantics=tyu.model/linux-x86_64-hosted/1".into(),
        format!("--concurrency={concurrency}"),
    ];
    if proven {
        args.push("--verify-policy=proven".into());
    }
    let verdict_file = dir.join("out.v2.json");
    args.push(format!("--verdicts={}", verdict_file.display()));
    args.push(format!(
        "--bind-obl={}",
        dir.join("Conc.obl.json").display()
    ));
    args.push(
        root.join("crates/execution-tests/fixtures/conc_roundtrip.mod")
            .display()
            .to_string(),
    );
    let out = Command::new(common::bin::resolve("langc"))
        .args(&args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The discharged-`proof` ids in the echo (`<out>/Conc.verdicts.inTree.json`).
fn echo_proof_ids(dir: &Path) -> Vec<String> {
    let echo_path = dir.join("out/Conc.verdicts.inTree.json");
    let bytes = fs::read(&echo_path).unwrap_or_default();
    match verifier::verdict::read_echo(&bytes) {
        Ok(echo) => echo
            .verdicts
            .records
            .iter()
            .filter(|r| !r.status.is_open() && r.trust == verifier::verdict::Trust::Proof)
            .map(|r| r.id.clone())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The echo's stale-verdicts count (E6421 accounting).
fn echo_stale(dir: &Path) -> u32 {
    let echo_path = dir.join("out/Conc.verdicts.inTree.json");
    let bytes = fs::read(&echo_path).unwrap_or_default();
    match verifier::verdict::read_echo(&bytes) {
        Ok(echo) => echo.stale_verdicts,
        Err(_) => 0,
    }
}

/// The Rust mirror of the abstract-atomic services trace is the codec's
/// `verifier::svcvec::run_trace` (the hand-rolled, schema-checked,
/// size-capped `tyu.svcvec/1` reader — same discipline as `tyu.vec/1` /
/// `tyu.fragvec/1`). Replay the committed corpus; returns divergences.
fn rust_svc_corpus(root: &Path) -> Vec<String> {
    let text =
        fs::read_to_string(root.join("sysroot/x86_64-unknown-linux-gnu/evidence/vectors.json"))
            .unwrap();
    let file =
        verifier::svcvec::parse_svcvec(text.as_bytes()).expect("svcvec corpus parses schema-exact");
    let mut divergences = Vec::new();
    for script in &file.scripts {
        match verifier::svcvec::run_trace(&script.ops) {
            Some(got) if got == script.expect => {}
            Some(got) => divergences.push(format!(
                "script '{}': got {got:?} want {:?} (Rust mirror)",
                script.id, script.expect
            )),
            None => divergences.push(format!("script '{}': blocked (Rust mirror)", script.id)),
        }
    }
    divergences
}

/// The port's `conformance --level services` replay of the committed corpus:
/// returns the `RESULT: services=… mismatches=…` tail line.
fn lean_svc_result(root: &Path) -> String {
    let exe = root.join(PORT).join(".lake/build/bin/conformance");
    if !exe.is_file() {
        Command::new("lake")
            .current_dir(root.join(PORT))
            .args(["build", "conformance"])
            .status()
            .expect("lake build conformance");
    }
    let out = Command::new(&exe)
        .args(["--level", "services", "--corpus"])
        .arg(root.join("sysroot/x86_64-unknown-linux-gnu/evidence"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "conformance --level services failed:\n{text}"
    );
    text.lines()
        .rev()
        .find(|l| l.starts_with("RESULT: services="))
        .unwrap_or_default()
        .to_string()
}

/// Build a hosted-source generator↔fixture byte-pin: every corpus script
/// must lower to the committed wire fixture EXACTLY
/// (`verifier::svcvec::render_hosted_source` — the deterministic source a
/// developer can write, which the hosted leg compiles and runs). The map
/// mirrors the execution-test's (script id → fixture file).
fn hosted_fixture_pin(root: &Path) -> Result<(), String> {
    const FIXTURES: &[(&str, &str)] = &[
        ("fifo-send-recv", "svc_fifo_send_recv.mod"),
        ("fifo-order", "svc_fifo_order.mod"),
        ("fifo-two-channels", "svc_fifo_two_channels.mod"),
        ("fifo-deep-isolation", "svc_fifo_deep_isolation.mod"),
    ];
    let text =
        fs::read_to_string(root.join("sysroot/x86_64-unknown-linux-gnu/evidence/vectors.json"))
            .map_err(|e| e.to_string())?;
    let corpus = verifier::svcvec::parse_svcvec(text.as_bytes()).map_err(|e| format!("{e:?}"))?;
    let mut problems = Vec::new();
    for script in &corpus.scripts {
        // Every committed script must be hostable (the shared-fn refusal is
        // the loud boundary — no script silently lacks a hosted counterpart).
        let generated = verifier::svcvec::render_hosted_source(script).ok_or_else(|| {
            format!(
                "script '{}' has no hosted source (refused by render_hosted_source)",
                script.id
            )
        })?;
        let fixture = FIXTURES
            .iter()
            .find(|(id, _)| *id == script.id)
            .map(|(_, f)| root.join(format!("crates/execution-tests/fixtures/{f}")))
            .ok_or_else(|| format!("script '{}' has no committed fixture mapping", script.id))?;
        let committed = fs::read_to_string(&fixture).map_err(|e| e.to_string())?;
        if committed != generated {
            problems.push(format!(
                "script '{}': committed fixture '{}' ≠ render_hosted_source output",
                script.id,
                fixture.display()
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

#[test]
fn service_relativism_and_template_harvest() {
    if !toolchain_present() {
        if std::env::var("TYU_SERVICE_E2E").is_ok() {
            panic!("TYU_SERVICE_E2E=1 requires `lean` and `lake` on PATH");
        }
        eprintln!(
            "skipping the service-relativism e2e (no Lean toolchain on PATH; ci/port.sh runs it)"
        );
        return;
    }
    let root = workspace_root();
    let dir = fresh_dir("main");
    fs::copy(
        root.join(FIXTURE).join("Conc.obl.json"),
        dir.join("Conc.obl.json"),
    )
    .unwrap();

    // 1. Relativism render (hosted = abstract-atomic): the service statement
    // renders; the gen.json header carries the concurrency declaration; the
    // renderer's hash byte-matches the Rust encoder's (the canonical
    // statement is independent of the concurrency declaration — the
    // relativism rides the artifact + FR-5 identity, not the canonical hash).
    render_fixture(&root, &dir, "Conc");
    let meta = fs::read_to_string(dir.join("Gen/Conc.gen.json")).unwrap();
    assert!(
        meta.contains("\"concurrency\": \"abstract-atomic\""),
        "gen.json must carry the concurrency declaration:\n{meta}"
    );
    let lean_text = fs::read_to_string(dir.join("Gen/Conc.lean")).unwrap();
    assert!(
        lean_text.contains("def stmt_Conc_roundtrip_pct_subtype_range_1"),
        "the service statement must render under abstract-atomic:\n{lean_text}"
    );
    assert!(
        lean_text.contains("Tyu.Services.traceInRange"),
        "the service statement must be over the abstract-atomic model:\n{lean_text}"
    );
    assert!(
        meta.contains("\"omitted\": true, \"reason\": \"opaque-site\""),
        "the opaque $top cast obligation must refuse honestly:\n{meta}"
    );
    // The Rust encoder's hash for the proven obligation (statement context is
    // unchanged by concurrency — the relativism gate is the render decision +
    // FR-5 identity).
    let byte_text = fs::read(dir.join("Conc.obl.json")).unwrap();
    let set = verifier::codec::read_obl(&byte_text).unwrap();
    assert_eq!(set.concurrency, "abstract-atomic");
    let o = set.obligations.iter().find(|o| o.id == OBL_ID).unwrap();
    let word_ir = set
        .facts
        .words
        .iter()
        .find(|w| w.name == o.site.word)
        .map(|w| w.ir.as_str())
        .unwrap_or("");
    let ctx = verifier::stmt::StatementContext::for_obligation(
        &set.module,
        &set.target,
        &set.model_semantics,
        &verifier::stmt::sha256_hex16(word_ir.as_bytes()),
        o,
    );
    let expected = ctx.statement_hash_hex(&o.formula);
    assert!(
        meta.contains(&format!("\"statement_hash\": \"{expected}\"")),
        "the renderer's statement hash must byte-match the Rust encoder's:\n{meta}"
    );

    // 2. Relativism FAIL-CLOSED: the same artifact under `unmodeled`
    // concurrency renders NOTHING — every obligation is `service-unmodeled`
    // (§Q14: concurrency-enabled statements under an unmodeled-services
    // bundle are unprovable, fail-closed open).
    let dir_nr = fresh_dir("unmodeled");
    fs::copy(
        root.join(FIXTURE).join("Conc-unmodeled.obl.json"),
        dir_nr.join("Conc.obl.json"),
    )
    .unwrap();
    render_fixture(&root, &dir_nr, "Conc");
    let meta_nr = fs::read_to_string(dir_nr.join("Gen/Conc.gen.json")).unwrap();
    assert!(
        meta_nr.contains("\"concurrency\": \"unmodeled\""),
        "{meta_nr}"
    );
    assert!(
        meta_nr.contains("\"reason\": \"service-unmodeled\""),
        "an unmodeled-services bundle must refuse service statements with \
         `service-unmodeled`:\n{meta_nr}"
    );
    let lean_nr = fs::read_to_string(dir_nr.join("Gen/Conc.lean")).unwrap();
    assert!(
        !lean_nr.contains("def stmt_Conc_"),
        "no statements may render under unmodeled concurrency:\n{lean_nr}"
    );

    // 3. The template harvests a `proof` (P15.2): the temp package proves the
    // FIFO round-trip statement; the proven obligation is `proof` +
    // `certificate`, the opaque one is `open` with the refusal witness.
    let pkg = fresh_dir("pkg");
    fs::copy(
        root.join(FIXTURE).join("Conc.obl.json"),
        pkg.join("Conc.obl.json"),
    )
    .unwrap();
    assemble_package(
        &root,
        &pkg,
        "Conc",
        &fs::read_to_string(root.join(FIXTURE).join("ConcFix.lean")).unwrap(),
    );
    let verdicts = run_harvest(&pkg);
    let v: verifier::verdict::Verdicts =
        verifier::verdict::read_verdicts(verdicts.as_bytes()).unwrap();
    assert_eq!(v.model_semantics, MODEL);
    assert_eq!(
        v.concurrency, "abstract-atomic",
        "the harvest doc must carry the concurrency declaration"
    );
    let rec = v
        .lookup(OBL_ID, "6a5be83a76c68a24")
        .expect("the proven obligation's record");
    assert_eq!(rec.status, verifier::verdict::VerdictStatus::Discharged);
    assert_eq!(rec.trust, verifier::verdict::Trust::Proof);
    assert_eq!(rec.method, Some(verifier::verdict::Method::Certificate));
    let opaque = v
        .lookup(OPAQUE_ID, "6a5be93a76c68bd7")
        .expect("the opaque obligation's record");
    assert!(opaque.status.is_open());
    assert_eq!(
        opaque.witness_reason.as_deref(),
        Some("opaque-site")
    );

    // 4. FR-5 consumption: the same provenance concurrency declaration
    // discharges; a DIFFERENT declaration is E6421-stale (fail-closed to
    // open, check retained — never a wrong discharge).
    let verdicts_doc = pkg.join("out.v2.json"); // written by run_harvest
    let consume_dir = fresh_dir("consume_aa");
    fs::copy(&verdicts_doc, consume_dir.join("out.v2.json")).unwrap();
    fs::copy(
        root.join(FIXTURE).join("Conc.obl.json"),
        consume_dir.join("Conc.obl.json"),
    )
    .unwrap();
    let (ok_aa, err_aa) = compile_conc(&root, &consume_dir, "abstract-atomic", false);
    assert!(
        ok_aa,
        "consumption under the matching concurrency must succeed: {err_aa}"
    );
    assert!(
        echo_proof_ids(&consume_dir).contains(&OBL_ID.to_string()),
        "the proven site must discharge under the matching concurrency"
    );
    assert_eq!(echo_stale(&consume_dir), 0);
    let consume_nr = fresh_dir("consume_unmodeled");
    fs::copy(&verdicts_doc, consume_nr.join("out.v2.json")).unwrap();
    fs::copy(
        root.join(FIXTURE).join("Conc.obl.json"),
        consume_nr.join("Conc.obl.json"),
    )
    .unwrap();
    let (ok_nr, err_nr) = compile_conc(&root, &consume_nr, "unmodeled", false);
    assert!(
        ok_nr,
        "a mismatched concurrency must NOT fail the build (checks retained): {err_nr}"
    );
    assert!(
        !echo_proof_ids(&consume_nr).contains(&OBL_ID.to_string()),
        "a verdict produced under a different concurrency declaration must not \
         discharge (E6421 stale, fail-closed to open)"
    );
    assert_eq!(
        echo_stale(&consume_nr),
        1,
        "the stale record must be counted"
    );

    // 5. Service conformance consensus (P15.2): the Rust codec mirror and the
    // port's `--level services` replay the committed svcvec corpus with ZERO
    // divergence (FIFO/atomicity asserted on two surfaces), the codec's
    // schema check fails closed on a renamed schema, and every corpus script
    // lowers EXACTLY to its committed hosted fixture (the wire-form the
    // execution-test hosted leg compiles + runs — direct script⇄runtime
    // differentials, not transitive agreement).
    let rust_div = rust_svc_corpus(&root);
    assert!(
        rust_div.is_empty(),
        "Rust mirror divergences:\n{}",
        rust_div.join("\n")
    );
    let lean_result = lean_svc_result(&root);
    assert!(
        lean_result.contains("mismatches=0"),
        "Lean services conformance must be zero-divergence: {lean_result}"
    );
    // A renamed schema is rejected (the codec checks it — the ad-hoc parser
    // the old mirror used could not).
    let svc_text =
        fs::read_to_string(root.join("sysroot/x86_64-unknown-linux-gnu/evidence/vectors.json"))
            .unwrap();
    let renamed = svc_text.replace("tyu.svcvec/1", "tyu.svcvec/9");
    assert_eq!(
        verifier::svcvec::parse_svcvec(renamed.as_bytes()),
        Err(verifier::svcvec::SvCodecError::SchemaVersion {
            found: "tyu.svcvec/9".to_string()
        }),
        "a renamed svcvec schema must fail closed"
    );
    hosted_fixture_pin(&root)
        .unwrap_or_else(|e| panic!("hosted-source generator ↔ committed fixture pin: {e}"));
}
