//! The `verify_manifest` record codec round-trip (PLAN-VERIFY-3 P11).
//!
//! Encode (lmod-pack) → scan (lmod) → digest recompute, obligation lookup,
//! and the JSON CLI summary path. Tampering and closed-set violations fail
//! closed at the *scanner* (the encoder refuses them first).

use lmod_pack::verify::{encode_verify_manifest, ObligationSpec, VerifyManifestSpec};
use sha2::Digest as _;

fn spec() -> VerifyManifestSpec {
    VerifyManifestSpec {
        semantics: "tyu.ir-sem/1.0".to_string(),
        stmt: "tyu.stmt/1.0".to_string(),
        target: "x86_64-unknown-none".to_string(),
        model: "tyu.model/x86_64-unknown-none/1".to_string(),
        policy: lmod::verify_manifest::VM_POLICY_PROVEN,
        certifier_class: lmod::verify_manifest::VM_CERTIFIER_PORT,
        certifier_name: "lean".to_string(),
        certifier_recognition: "tyu-port/lean/1".to_string(),
        candidate_ratio: 0,
        counts: [3, 0, 0, 1],
        obligations: vec![
            ObligationSpec {
                id: "Bank::withdraw::contract-post::0".to_string(),
                id_hash: 0x1122_3344_5566_7788,
                status: lmod::verify_manifest::VM_STATUS_DISCHARGED,
                trust: lmod::verify_manifest::VM_TRUST_PROOF,
                statement_hash: [7u8; 32],
            },
            ObligationSpec {
                id: "Bank::withdraw::subtype-range::0".to_string(),
                id_hash: 0xaabb_ccdd_0011_2233,
                status: lmod::verify_manifest::VM_STATUS_OPEN,
                trust: lmod::verify_manifest::VM_TRUST_OPEN,
                statement_hash: [9u8; 32],
            },
        ],
    }
}

#[test]
fn encode_scan_roundtrip() {
    let record = encode_verify_manifest(&spec()).expect("encode");
    assert!(record.len() >= 8, "record must carry size+tag");

    let mut payload = b"prior-modinfo-bytes".to_vec();
    payload.extend_from_slice(&record);
    let vm = lmod::verify_manifest::scan_verify_manifest(&payload)
        .expect("scan ok")
        .expect("record present");

    assert_eq!(vm.semantics, b"tyu.ir-sem/1.0");
    assert_eq!(vm.stmt, b"tyu.stmt/1.0");
    assert_eq!(vm.target, b"x86_64-unknown-none");
    assert_eq!(vm.model, b"tyu.model/x86_64-unknown-none/1");
    assert_eq!(vm.policy, lmod::verify_manifest::VM_POLICY_PROVEN);
    assert_eq!(vm.certifier_class, lmod::verify_manifest::VM_CERTIFIER_PORT);
    assert_eq!(vm.certifier_name, b"lean");
    assert_eq!(vm.certifier_recognition, b"tyu-port/lean/1");
    assert_eq!(vm.count, 2);
    assert_eq!(vm.counts, [3, 0, 0, 1]);

    // digest recompute over the obligations region
    let region = &payload[vm.obligations_start..vm.obligations_end];
    let digest: [u8; 32] = sha2::Sha256::digest(region).into();
    assert_eq!(digest, vm.digest, "digest must recompute over the region");

    // obligation accessors + binary search over the sorted region
    let first = lmod::verify_manifest::obligation(&vm, &payload, 0).expect("obligation 0");
    assert_eq!(first.id, b"Bank::withdraw::contract-post::0");
    assert_eq!(first.id_hash, 0x1122_3344_5566_7788);
    assert_eq!(first.status, lmod::verify_manifest::VM_STATUS_DISCHARGED);
    assert_eq!(first.trust, lmod::verify_manifest::VM_TRUST_PROOF);
    assert_eq!(first.statement_hash, [7u8; 32]);
    let second = lmod::verify_manifest::obligation(&vm, &payload, 1).expect("obligation 1");
    assert_eq!(second.id, b"Bank::withdraw::subtype-range::0");
    let idx =
        lmod::verify_manifest::find_obligation(&vm, &payload, b"Bank::withdraw::subtype-range::0");
    assert_eq!(idx, Some(1));
    assert_eq!(
        lmod::verify_manifest::find_obligation(&vm, &payload, b"nope"),
        None
    );
}

#[test]
fn encoder_sorts_obligations() {
    // Feed the encoder unsorted ids; it must canonicalize.
    let mut s = spec();
    s.obligations.swap(0, 1);
    let record = encode_verify_manifest(&s).expect("encode");
    let mut payload = Vec::new();
    payload.extend_from_slice(&record);
    let vm = lmod::verify_manifest::scan_verify_manifest(&payload)
        .expect("scan")
        .expect("present");
    let a = lmod::verify_manifest::obligation(&vm, &payload, 0).expect("0");
    let b = lmod::verify_manifest::obligation(&vm, &payload, 1).expect("1");
    assert!(a.id <= b.id, "encoder must sort by id");
}

#[test]
fn encoder_rejects_out_of_closed_sets() {
    let mut s = spec();
    s.policy = 55;
    assert!(encode_verify_manifest(&s).is_err());
    let mut s2 = spec();
    s2.obligations[0].trust = 9;
    assert!(encode_verify_manifest(&s2).is_err());
    let mut s3 = spec();
    s3.obligations[0].id = "x".repeat(513);
    assert!(encode_verify_manifest(&s3).is_err());
}

#[test]
fn json_summary_path() {
    let json = r#"{
      "schema": "tyu.vm/1",
      "semantics": "tyu.ir-sem/1.0",
      "stmt": "tyu.stmt/1.0",
      "target": "x86_64-unknown-none",
      "model": "unmodeled",
      "policy": "no-open",
      "certifier": {"class": "port", "name": "lean", "recognition": "tyu-port/lean/1"},
      "candidate_ratio": 0,
      "counts": {"proof": 1, "checked": 0, "assumed": 0, "open": 0},
      "obligations": [
        {"id": "A::w::contract-post::0", "id_hash": 42,
         "status": "discharged", "trust": "proof",
         "statement_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
      ]
    }"#;
    let spec = lmod_pack::verify::verify_manifest_from_json(json).expect("parse");
    assert_eq!(spec.target, "x86_64-unknown-none");
    assert_eq!(spec.model, "unmodeled");
    assert_eq!(spec.policy, lmod::verify_manifest::VM_POLICY_NO_OPEN);
    assert_eq!(spec.certifier_name, "lean");
    assert_eq!(spec.counts, [1, 0, 0, 0]);
    assert_eq!(spec.obligations.len(), 1);
    assert_eq!(spec.obligations[0].statement_hash, [0xaa; 32]);
    // And the encoded record scans clean.
    let record = encode_verify_manifest(&spec).expect("encode");
    let mut payload = Vec::new();
    payload.extend_from_slice(&record);
    let vm = lmod::verify_manifest::scan_verify_manifest(&payload)
        .unwrap()
        .unwrap();
    let region = &payload[vm.obligations_start..vm.obligations_end];
    assert_eq!(sha2::Sha256::digest(region).as_slice(), &vm.digest[..]);
}

#[test]
fn bad_json_summary_rejected() {
    assert!(lmod_pack::verify::verify_manifest_from_json("not json").is_err());
    assert!(lmod_pack::verify::verify_manifest_from_json(r#"{"schema":"tyu.stmt/1.0"}"#).is_err());
    assert!(lmod_pack::verify::verify_manifest_from_json(
        r#"{"schema":"tyu.vm/1","policy":"heavenly"}"#
    )
    .is_err());
}
