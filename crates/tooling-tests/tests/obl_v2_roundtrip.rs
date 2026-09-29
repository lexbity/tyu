//! Obligation-artifact v2 round-trip and schema-conformance tests
//! (PLAN-VERIFY-3 P1.2).
//!
//! - every positive corpus fixture, every supported triple: `langc
//!   --emit=obligations` → read → re-encode → byte-exact (Q11 determinism
//!   survives the v1→v2 growth);
//! - the v2 identity fields (`stmt`, `target`, `platform`, `model_semantics`)
//!   survive the round trip and are populated by the driver;
//! - unknown (future/additive) keys are skipped, not failed;
//! - a word whose IR exceeds the 8 KiB per-word cap fails closed at encode
//!   (E6401-class `WordIrTooLarge`).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use verifier::codec::{encode_obl, read_obl, CodecError};

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

/// The positive corpus fixtures: everything `--emit=obligations` must build
/// and round-trip. Error fixtures (e50xx) are negative tests — they must NOT
/// produce artifacts.
const POSITIVE_FIXTURES: &[(&str, &str)] = &[
    ("clean", "ci/verify-corpus/clean.mod"),
    ("contract", "ci/verify-corpus/contract.mod"),
    ("event-loop", "ci/verify-corpus/event-loop.mod"),
    ("open-cast", "ci/verify-corpus/open-cast.mod"),
];

/// Every triple the statement pipeline is parameterized over (PLAN-VERIFY-3
/// §Q3): extracting obligations for each is assembler-free, so all four are
/// legal `--emit=obligations` targets.
const TRIPLES: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-none",
    "armv7m-unknown-none",
    "riscv32-unknown-none",
];

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_obl_v2_roundtrip")
        .join(format!(
            "{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn compile_obl(dir: &Path, source_path: &Path, target: &str) -> Vec<u8> {
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--target={target}"))
        .arg(format!("--out-dir={}", dir.display()))
        .arg(source_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "langc --emit=obligations failed for {} ({}): {}",
        source_path.display(),
        target,
        String::from_utf8_lossy(&out.stderr)
    );
    // The artifact file is named after the DECLARED module, not the file stem.
    let artifact = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".obl.json"))
        })
        .unwrap_or_else(|| panic!("no *.obl.json produced in {}", dir.display()));
    std::fs::read(artifact).unwrap()
}

#[test]
fn corpus_roundtrips_byte_exact_across_all_triples() {
    for &(name, fixture) in POSITIVE_FIXTURES {
        for &target in TRIPLES {
            let dir = fresh_dir(&format!("{name}-{target}"));
            let bytes = compile_obl(&dir, &common::workspace_root().join(fixture), target);
            let set = read_obl(&bytes).expect("artifact must parse");
            let reencoded = encode_obl(&set).expect("re-encode");
            assert_eq!(
                bytes, reencoded,
                "read → re-encode must be byte-exact for {name} on {target}"
            );
            assert_eq!(set.schema, "tyu.obl/v2");
            assert_eq!(set.target, target, "artifact must record its triple");
            assert_eq!(set.platform, target);
            assert_eq!(
                set.model_semantics,
                verifier::model::MODEL_UNMODELED,
                "no bundle model is wired before P12"
            );
            assert_eq!(set.stmt, verifier::stmt::STMT_SCHEMA);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[test]
fn v2_obligations_carry_intent_and_word_ir() {
    // The contract fixture has obligations; each must carry a synthesized
    // intent (authored=false goes through the driver) and its word facts must
    // carry the canonical op-text + block count.
    let dir = fresh_dir("fields");
    let bytes = compile_obl(
        &dir,
        &common::workspace_root().join("ci/verify-corpus/contract.mod"),
        "x86_64-unknown-linux-gnu",
    );
    let set = read_obl(&bytes).expect("parse");
    assert!(
        !set.obligations.is_empty(),
        "contract corpus fixture must produce obligations"
    );
    let words_with_ir = set
        .facts
        .words
        .iter()
        .filter(|w| !w.ir.is_empty() && w.blocks > 0)
        .count();
    assert!(
        words_with_ir > 0,
        "word facts must carry the canonical op-text and block count"
    );
    for o in &set.obligations {
        assert!(
            !o.intent.label.is_empty(),
            "every obligation must carry a synthesized or authored intent"
        );
        if let Some(w) = set.facts.words.iter().find(|w| w.name == o.site.word) {
            assert_eq!(
                verifier::stmt::sha256_hex16(w.ir.as_bytes()).len(),
                16,
                "word_ir_hash is the 16-hex SHA-256 prefix"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reader_tolerates_additive_keys_on_v2() {
    let dir = fresh_dir("additive");
    let bytes = compile_obl(
        &dir,
        &common::workspace_root().join("ci/verify-corpus/contract.mod"),
        "x86_64-unknown-linux-gnu",
    );
    let mut text = String::from_utf8(bytes).unwrap();
    // Future-style top-level key; the reader must skip it (additive growth).
    text = text.replace(",\"facts\"", ",\"future_key\":{\"nested\":[1,2]},\"facts\"");
    let parsed = read_obl(text.as_bytes()).expect("additive keys are skipped");
    assert_eq!(parsed.module, "Contract");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The per-word IR cap (8 KiB, E6401-class) fails closed at encode — a word
/// past the bound is an artifact the statement pipeline cannot carry.
/// (The construction-level test lives in `verifier`'s codec unit tests, where
/// the `ir` crate is a direct dependency; here the property is re-asserted
/// through the public codec surface against a string-shaped WordFact built
/// via the model's plain API.)
#[test]
fn word_ir_cap_fails_closed() {
    // A WordFact with an oversized ir string cannot even be constructed
    // through the plain API path — fabricate one and exercise the codec
    // directly: oversized word IR is caught by `encode_obl`.
    let set = verifier::model::OblSet {
        schema: "tyu.obl/v2".to_string(),
        semantics: verifier::semantics::SEMANTICS_VERSION.to_string(),
        stmt: verifier::stmt::STMT_SCHEMA.to_string(),
        module: "Bank".to_string(),
        target: String::new(),
        platform: String::new(),
        model_semantics: verifier::model::MODEL_UNMODELED.to_string(),
        concurrency: verifier::model::CONCURRENCY_UNMODELED.to_string(),
        abi_contract_version: 2,
        facts: verifier::model::Facts {
            words: vec![verifier::model::WordFact {
                name: "big".to_string(),
                net: 0,
                high: 1,
                top: false,
                performs: vec![],
                diverge_free: true,
                ir: "x".repeat(verifier::model::WORD_IR_MAX_BYTES + 1),
                blocks: 1,
            }],
            subtypes: vec![],
            predicates: vec![],
        },
        obligations: vec![],
    };
    let err = encode_obl(&set).expect_err("oversized word IR must fail closed");
    assert!(matches!(err, CodecError::WordIrTooLarge { .. }));
    assert_eq!(err.code(), 6401);
}

/// A v1 artifact (pre-P1.2 schema) is rejected with E6400 — the compat break
/// is owned by §13, never a silent best-effort read.
#[test]
fn v1_artifact_is_rejected_as_wrong_schema() {
    let dir = fresh_dir("v1");
    // Hand-roll a v1-shaped artifact: correct shape, old schema string.
    let v1 = br#"{"schema":"tyu.obl/v1","semantics":"tyu.ir-sem/1.0","module":"Bank","abi_contract_version":2,"facts":{"words":[],"subtypes":[]},"obligations":[]}"#;
    let err = read_obl(v1).expect_err("v1 must be rejected");
    match err {
        CodecError::SchemaVersion { ref found } => {
            assert_eq!(found, "tyu.obl/v1");
            assert_eq!(err.code(), 6400);
        }
        other => panic!("expected a schema-version rejection, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Band-rule stability (§Q4 item 3) at the mechanism level: recompiling the
/// same module yields identical statement hashes per obligation, and ANY
/// perturbation of the canonical inputs (a weakened formula — the tamper the
/// band rule exists to catch) changes the hash. P7's E6421 consumption
/// recomputes and fails closed on exactly this signal.
#[test]
fn statement_hashes_stable_and_tamper_detected() {
    let dir = fresh_dir("stmt-stable");
    let bytes_a = compile_obl(
        &dir,
        &common::workspace_root().join("ci/verify-corpus/contract.mod"),
        "x86_64-unknown-linux-gnu",
    );
    let set_a = read_obl(&bytes_a).expect("parse");
    // Recompile → identical statement hashes (determinism + band stability).
    let bytes_b = compile_obl(
        &dir,
        &common::workspace_root().join("ci/verify-corpus/contract.mod"),
        "x86_64-unknown-linux-gnu",
    );
    let set_b = read_obl(&bytes_b).expect("parse");

    let hashes = |set: &verifier::model::OblSet| -> Vec<String> {
        let mut out = Vec::new();
        for o in &set.obligations {
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
            out.push(ctx.statement_hash_hex(&o.formula));
        }
        out
    };
    assert_eq!(
        hashes(&set_a),
        hashes(&set_b),
        "same input ⇒ same statement hashes"
    );

    // Tamper: widen a range in the first obligation's formula (a weakened
    // statement). The recomputed hash MUST change.
    let mut mutated = set_a.clone();
    let o = &mut mutated.obligations[0];
    match &mut o.formula {
        verifier::model::Formula::InRange { hi, .. } => *hi = 4_000_000,
        other => panic!("expected InRange, got {other:?}"),
    }
    let wi = mutated
        .facts
        .words
        .iter()
        .find(|w| w.name == o.site.word)
        .map(|w| verifier::stmt::sha256_hex16(w.ir.as_bytes()))
        .unwrap_or_default();
    let ctx = verifier::stmt::StatementContext::for_obligation(
        &mutated.module,
        &mutated.target,
        &mutated.model_semantics,
        &wi,
        o,
    );
    assert_ne!(
        ctx.statement_hash_hex(&o.formula),
        hashes(&set_a)[0],
        "a weakened statement must change the statement hash"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
