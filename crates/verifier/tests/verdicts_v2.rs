//! `tyu.verdicts/v2` codec tests (PLAN-VERIFY-3 P7.2): the v2 record surface
//! (trust/method/surface/statement_hash/authored/proof/claimed), the closed
//! registries (E6417), the `proof_ref` retirement, the producer-recognition
//! downgrade (§Q6), the `ParseSurface` consolidation, and deterministic
//! emission. Everything here is hand-rolled-JSON-encoded and -parsed through
//! the public codec surface.

use verifier::verdict::{
    read_verdicts, Authored, Certifier, Method, ParseSurface, ProofInfo, ProofKind, ProofSurface,
    ToolIdentity, Trust, VerdictRecord, VerdictStatus, Verdicts, RECOGNIZED_CERTIFIERS,
};

fn certifier(recognition: &str) -> Certifier {
    Certifier {
        class: "port".to_string(),
        name: "lean".to_string(),
        recognition: recognition.to_string(),
        tool: ToolIdentity {
            name: "harvest".to_string(),
            version: "0.1.0".to_string(),
        },
        toolchain: "lean4:4.27.0".to_string(),
    }
}

fn full_record() -> VerdictRecord {
    VerdictRecord {
        id: "Bank::withdraw::contract-post::0".to_string(),
        id_hash: "0123456789abcdef".to_string(),
        status: VerdictStatus::Discharged,
        trust: Trust::Proof,
        method: Some(Method::Certificate),
        surface: Some(ProofSurface::Ir),
        statement_hash: Some("a".repeat(64)),
        authored: Some(Authored::Developer),
        proof: Some(ProofInfo {
            kind: ProofKind::Certificate,
            statement: "tyu.stmt/1.0".to_string(),
            theorem: Some("obl_Bank_withdraw_contract_post_0".to_string()),
            kernel_check: Some("lean-kernel+lean4checker".to_string()),
            file: Some("proofs/Bank.lean".to_string()),
            relies: Some(vec!["T-S".to_string()]),
        }),
        claimed: None,
        justification: None,
        witness_reason: None,
        note: None,
    }
}

fn doc(records: Vec<VerdictRecord>) -> String {
    let bytes = verifier::verdict::encode_verdicts(
        "harvest",
        "0.1.0",
        Some(&certifier(RECOGNIZED_CERTIFIERS[0])),
        "x86_64-unknown-none",
        "tyu.model/x86_64-unknown-none/1",
        &records,
        0,
        &Default::default(),
    )
    .expect("encode");
    String::from_utf8(bytes).unwrap()
}

#[test]
fn v2_record_round_trips_every_field() {
    let rec = full_record();
    let text = doc(vec![rec.clone()]);
    assert!(text.contains("\"schema\":\"tyu.verdicts/v2\""), "{text}");
    // The v1 dangling promise is retired: v2 must never emit `proof_ref`.
    assert!(
        !text.contains("proof_ref"),
        "proof_ref must be retired: {text}"
    );
    let back = read_verdicts(text.as_bytes()).expect("parse");
    assert_eq!(back.records.len(), 1);
    assert_eq!(back.records[0], rec);
    assert_eq!(back.target, "x86_64-unknown-none");
    assert_eq!(
        back.certifier.as_ref().unwrap().recognition,
        RECOGNIZED_CERTIFIERS[0]
    );
}

#[test]
fn emission_is_deterministic_and_sorted() {
    let recs: Vec<VerdictRecord> = ["Bank::a::k::0", "Bank::b::k::1", "Bank::m::k::2"]
        .iter()
        .enumerate()
        .map(|(i, id)| {
            VerdictRecord::discharged(
                id.to_string(),
                format!("{i:016x}"),
                Trust::Checked,
                Method::Interval,
            )
        })
        .collect();
    let a = doc(recs.clone());
    let b = doc(recs);
    assert_eq!(a, b, "byte-identical across runs (FR-16/17)");
    // Sorted ids on the read side.
    let parsed = read_verdicts(a.as_bytes()).unwrap();
    let ids: Vec<&str> = parsed.records.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["Bank::a::k::0", "Bank::b::k::1", "Bank::m::k::2"]);
}

#[test]
fn open_records_are_admitted_on_both_surfaces() {
    // The ParseSurface consolidation: v2 verdicts are FULL reports — the
    // harvest's `open` records (with witnesses) are consumed on the input
    // surface too (they close nothing; absence is open anyway). The echo
    // surface parses the same records plus its members.
    let body = br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"","model_semantics":"","verdicts":[{"id":"x","id_hash":"0","status":"open","trust":"open","witness":{"reason":"unproven"}}]}"#;
    let v = read_verdicts(body).expect("input admits open");
    assert!(v.records[0].status.is_open());
    assert_eq!(ParseSurface::Input, ParseSurface::Input);
    let echo = verifier::verdict::read_echo(body).expect("echo admits open");
    assert!(echo.verdicts.records[0].status.is_open());
}

#[test]
fn unknown_method_and_kind_are_e6417() {
    let err = read_verdicts(
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"","model_semantics":"","verdicts":[{"id":"x","id_hash":"0","status":"discharged","trust":"proof","method":"quantum"}]}"#,
    )
    .expect_err("unknown method must fail");
    assert_eq!(err.code(), 6417, "E6417 on unknown method: {err:?}");

    let err = read_verdicts(
        br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"","model_semantics":"","verdicts":[{"id":"x","id_hash":"0","status":"discharged","trust":"proof","method":"certificate","statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","proof":{"kind":"quantum","statement":"s"}}]}"#,
    )
    .expect_err("unknown proof.kind must fail");
    assert_eq!(err.code(), 6417, "E6417 on unknown proof.kind: {err:?}");
}

#[test]
fn downgrade_preserves_claimed_for_unrecognized_producers() {
    // A recognized producer's certificates survive.
    let rec = full_record(); // trust=proof, method=certificate, proof object + hash present
    let recognized: Verdicts = read_verdicts(doc(vec![rec.clone()]).as_bytes()).unwrap();
    assert!(recognized.producer_recognized());
    let kept = recognized.restrict_to_recognized();
    assert_eq!(kept.records[0].trust, Trust::Proof);

    // Unrecognized: proof downgrades to assumed; the claim is preserved.
    let body = verifier::verdict::encode_verdicts(
        "harvest",
        "0.1.0",
        Some(&certifier("tyu-port/unknown/9")),
        "x86_64-unknown-none",
        "unmodeled",
        &[rec],
        0,
        &Default::default(),
    )
    .unwrap();
    let v: Verdicts = read_verdicts(&body).unwrap();
    assert!(!v.producer_recognized());
    let v = v.restrict_to_recognized();
    assert_eq!(v.records[0].status, VerdictStatus::Assumed);
    assert_eq!(v.records[0].trust, Trust::Assumed);
    let claimed = v.records[0].claimed.as_ref().expect("claimed preserved");
    assert_eq!(claimed.trust.as_deref(), Some("proof"));
}

#[test]
fn checked_survives_the_round_trip_of_unrecognized_files() {
    // The toolchain's own `checked` records ride langc's echo round-trip even
    // when the echo's in-tree certifier is not a port (never downgraded).
    let rec = VerdictRecord::discharged("x".into(), "0".into(), Trust::Checked, Method::Interval);
    let body = verifier::verdict::encode_verdicts(
        "langc",
        "0.1.0",
        Some(&certifier("-")),
        "x86_64-unknown-none",
        "unmodeled",
        &[rec],
        0,
        &Default::default(),
    )
    .unwrap();
    let v: Verdicts = read_verdicts(&body).unwrap().restrict_to_recognized();
    assert_eq!(v.records[0].trust, Trust::Checked);
}

#[test]
fn certificate_requires_statement_hash_and_proof_object() {
    // §6.3: certificate/rederive without statement_hash → malformed; proof
    // without the proof object → malformed.
    let no_hash = br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"","model_semantics":"","verdicts":[{"id":"x","id_hash":"0","status":"discharged","trust":"proof","method":"certificate","proof":{"kind":"certificate","statement":"s"}}]}"#;
    assert!(read_verdicts(no_hash).is_err());
    let no_object = br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"","model_semantics":"","verdicts":[{"id":"x","id_hash":"0","status":"discharged","trust":"proof","method":"certificate","statement_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
    assert!(read_verdicts(no_object).is_err());
}

#[test]
fn discharged_without_trust_is_malformed() {
    let body = br#"{"schema":"tyu.verdicts/v2","certifier":null,"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"","model_semantics":"","verdicts":[{"id":"x","id_hash":"0","status":"discharged"}]}"#;
    let err = read_verdicts(body).expect_err("trust required on discharged");
    assert_eq!(err.code(), 6402, "err: {err:?}");
}

#[test]
fn stale_count_is_closed_to_obligation_ids() {
    let rec = VerdictRecord::discharged(
        "Bank::gone::k::0".into(),
        "0".into(),
        Trust::Checked,
        Method::Interval,
    );
    let v = read_verdicts(doc(vec![rec]).as_bytes()).unwrap();
    let obligations = vec![crate_obligation("Bank::here::k::0")];
    assert_eq!(v.stale_count(&obligations), 1);
}

/// A minimal obligation stub with the identity fields `stale_count` reads.
fn crate_obligation(id: &str) -> verifier::model::Obligation {
    verifier::model::Obligation {
        id: id.to_string(),
        id_hash: "0".to_string(),
        kind: verifier::model::Kind::SubtypeRange,
        site: verifier::model::Site {
            word: "k".to_string(),
            occurrence: 0,
            span: verifier::model::SpanInfo { line: 0, col: 0 },
        },
        intent: verifier::model::Intent {
            label: String::new(),
            subject: String::new(),
            authored: false,
        },
        formula: verifier::model::Formula::InRange {
            value: verifier::model::Oel::Var {
                name: "in.0".to_string(),
            },
            lo: 0,
            hi: 10,
        },
        assumptions: Vec::new(),
        cycles: Vec::new(),
        provenance: verifier::model::Provenance::Direct,
    }
}
