//! Codec round-trip and schema-conformance tests (static-verification.md
//! slice P2).
//!
//! - serialize → deserialize → serialize is byte-exact (Q11 determinism);
//! - the serialized document conforms to the §6.1 schema: every field
//!   present, exact key order, against a committed golden in
//!   `test-goldens/obl/codec-golden.json`;
//! - `read_obl` fail-closes on the schema/semantics/malformed/oversize
//!   classes (E6400/E6401 behavior).
//!
//! Bless a deliberate schema change with TYU_BLESS_OBL_GOLDEN=1 and review the
//! diff; a schema change MUST also bump `OBL_SCHEMA` (static-verification.md
//! §6.1 schema lifecycle, owner doc `verification-obligations.md`).

use std::fs;
use std::path::PathBuf;

use verifier::codec::{encode_obl, read_obl};
use verifier::model::{ExtractionCtx, Formula, Kind, Oel, Provenance};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The fixture obligation set: two words covering the C1 (param), C2
/// (return), and C3 (cast) subtype-range sites, plus word facts and one
/// subtype fact — the exact shapes langc will produce for the `bank` corpus
/// fixture.
fn sample_set() -> verifier::model::OblSet {
    let mut ctx = ExtractionCtx::new(b"Bank");

    // Word facts match the compiled `clamp`/`bounded_inc` words.
    ctx.push_subtype_fact(b"Percent", 0, 100);

    // clamp ( i64 -- Percent ): return check + one cast.
    ctx.begin_word(b"clamp");
    ctx.push_word_fact(
        b"clamp",
        ir::StackBound {
            net: 0,
            high: ir::High::Slots(4),
        },
        ir::EffectSet::empty(),
    );
    ctx.record(
        Kind::SubtypeRange,
        Formula::InRange {
            value: Oel::Var {
                name: "out.0".to_string(),
            },
            lo: 0,
            hi: 100,
        },
        0,
        0,
        Provenance::Direct,
        Vec::new(),
    );
    ctx.record(
        Kind::SubtypeRange,
        Formula::InRange {
            value: Oel::Cast {
                from: "i64".to_string(),
                to: "Percent".to_string(),
                arg: Box::new(Oel::Var {
                    name: "$top".to_string(),
                }),
            },
            lo: 0,
            hi: 100,
        },
        7,
        3,
        Provenance::Opaque,
        Vec::new(),
    );

    // reg_read ( u32 -- u32 ): an emulated-aperture access (P3) — the
    // OffsetLE head with the aperture-size descriptor assumption.
    ctx.begin_word(b"reg_read");
    ctx.push_word_fact(
        b"reg_read",
        ir::StackBound {
            net: 0,
            high: ir::High::Slots(2),
        },
        ir::EffectSet::from_bits(ir::EffectSet::MMIO),
    );
    let mut mmio_assumptions = Vec::new();
    mmio_assumptions.push(verifier::model::Assumption::ApertureSize {
        aperture: 0,
        size: 65536,
    });
    ctx.record(
        Kind::MmioBounds,
        Formula::OffsetLE {
            off: Some(0x1000),
            width: 4,
            size: 65536,
        },
        4,
        10,
        Provenance::Direct,
        mmio_assumptions,
    );
    // bounded_inc ( Percent -- Percent ): param check, cast, return check.
    ctx.begin_word(b"bounded_inc");
    ctx.push_word_fact(
        b"bounded_inc",
        ir::StackBound {
            net: 0,
            high: ir::High::Slots(3),
        },
        ir::EffectSet::empty(),
    );
    ctx.record(
        Kind::SubtypeRange,
        Formula::InRange {
            value: Oel::Var {
                name: "in.0".to_string(),
            },
            lo: 0,
            hi: 100,
        },
        0,
        0,
        Provenance::Direct,

        Vec::new(),
    );
    ctx.record(
        Kind::SubtypeRange,
        Formula::InRange {
            value: Oel::Cast {
                from: "i64".to_string(),
                to: "Percent".to_string(),
                arg: Box::new(Oel::Var {
                    name: "$top".to_string(),
                }),
            },
            lo: 0,
            hi: 100,
        },
        12,
        14,
        Provenance::Opaque,

        Vec::new(),
    );
    ctx.record(
        Kind::SubtypeRange,
        Formula::InRange {
            value: Oel::Var {
                name: "out.0".to_string(),
            },
            lo: 0,
            hi: 100,
        },
        0,
        0,
        Provenance::Direct,

        Vec::new(),
    );
    // Slice P6: a contract obligation — `PredicateHolds` transcluding the
    // callee's predicate (name + IR + hash) over caller-side args.
    ctx.record(
        Kind::ContractPre,
        Formula::PredicateHolds {
            pred: verifier::model::PredicateRef {
                module: "Bank".to_string(),
                name: "pct-in-range".to_string(),
                ir: vec![
                    "block b0".to_string(),
                    "dup Percent".to_string(),
                    "const_i64 0".to_string(),
                    "cmp_ge".to_string(),
                    "ret".to_string(),
                ],
                ir_hash: "11aa22bb33cc44dd".to_string(),
            },
            args: vec![Oel::Var {
                name: "$top".to_string(),
            }],
        },
        41,
        3,
        Provenance::Opaque,
        vec![verifier::model::Assumption::ContractPredicate {
            module: "Bank".to_string(),
            name: "pct-in-range".to_string(),
            ir_hash: "11aa22bb33cc44dd".to_string(),
        }],
    );

    ctx.into_set()
}

fn golden_path() -> PathBuf {
    workspace_root().join("test-goldens/obl/codec-golden.json")
}

#[test]
fn encode_decode_encode_is_byte_exact() {
    let set = sample_set();
    let first = encode_obl(&set).expect("encode");
    let decoded = read_obl(&first).expect("decode");
    assert_eq!(decoded, set, "decode must reproduce the model");
    let second = encode_obl(&decoded).expect("re-encode");
    assert_eq!(
        first, second,
        "encode -> decode -> encode must be byte-exact (Q11)"
    );
}

#[test]
fn golden_conformance_exact_bytes() {
    let bytes = encode_obl(&sample_set()).expect("encode");
    let golden = golden_path();
    if std::env::var_os("TYU_BLESS_OBL_GOLDEN").is_some() {
        if let Some(parent) = golden.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&golden, &bytes).unwrap();
        return;
    }
    let expected = fs::read(&golden).unwrap_or_else(|err| {
        panic!(
            "missing OBL golden {}: {err}; rerun with TYU_BLESS_OBL_GOLDEN=1",
            golden.display()
        )
    });
    assert_eq!(
        expected, bytes,
        "obl.json golden drifted; every structural change MUST bump OBL_SCHEMA"
    );
}

/// Slice P7: the image-verdicts record (Q5/FR-11, E6415) — the durable,
/// validated evidence of the two-pass guard-elision decision. Round-trips
/// byte-exactly, and a malformed/version-mismatched record is E6415
/// (fail-loud: the guards are never silently treated as retained off a
/// corrupt record).
#[test]
fn image_verdicts_roundtrip_and_fail_closed() {
    use verifier::codec::{encode_image_verdicts, read_image_verdicts, CodecError, ImageVerdicts};
    use verifier::report::MainContextAccounting;

    let rec = ImageVerdicts {
        elided: true,
        main: MainContextAccounting {
            high: 12,
            top: false,
            budget: 16384,
            verdict: "discharged".to_string(),
        },
    };
    let bytes = encode_image_verdicts(&rec).expect("encode");
    let back = read_image_verdicts(&bytes).expect("decode");
    assert_eq!(back, rec, "image verdicts must round-trip");
    assert!(bytes.starts_with(b"{\"schema\":\"tyu.image-verdicts/v1\",\"semantics\":\"tyu.ir-sem/1.0\",\"guards\":\"elided\""));

    // A retained record round-trips too (the refused-elision state).
    let retained = ImageVerdicts {
        elided: false,
        ..rec
    };
    let b2 = encode_image_verdicts(&retained).expect("encode");
    assert!(read_image_verdicts(&b2).expect("decode").elided == false);

    // Malformed input → E6415, never a silent default.
    let err = read_image_verdicts(b"{\"schema\":\"tyu.image-verdicts/v9\"}").unwrap_err();
    assert_eq!(err.code(), 6415);
    let err = read_image_verdicts(b"not json at all").unwrap_err();
    assert_eq!(err.code(), 6415);
    let err = read_image_verdicts(
        b"{\"schema\":\"tyu.image-verdicts/v1\",\"semantics\":\"tyu.ir-sem/1.0\",\"guards\":\"sometimes\"}",
    )
    .unwrap_err();
    assert!(matches!(err, CodecError::ImageVerdictsInvalid { .. }));
}

/// The golden document's top-level key order is pinned textually (not just by
/// byte equality): schema → semantics → module → abi_contract_version → facts
/// → obligations, and per-obligation id → id_hash → kind → site → formula →
/// assumptions → provenance.
#[test]
fn golden_key_order_is_schema_order() {
    let text = String::from_utf8(encode_obl(&sample_set()).expect("encode")).unwrap();
    let (top, obligations_rest) = text
        .split_once("\"obligations\":[{")
        .expect("obligations array present");
    // Top-level order check: schema → semantics → module → abi_contract_version
    // → facts(words, subtypes) — all before `obligations`.
    let top_needles = [
        "{\"schema\":\"tyu.obl/v1\"",
        "\"semantics\":",
        "\"module\":\"Bank\"",
        "\"abi_contract_version\":",
        "\"facts\":{\"words\":[",
        "\"subtypes\":[",
    ];
    let mut pos = 0usize;
    for needle in top_needles {
        let idx = top[pos..].find(needle).unwrap_or_else(|| {
            panic!("top-level key `{needle}` missing or out of order in: {top}")
        });
        pos += idx + needle.len();
    }
    // Per-obligation order check, on the first record: id → id_hash → kind →
    // site → formula → assumptions → provenance.
    let record = obligations_rest
        .split("},{\"id\"")
        .next()
        .expect("first obligation record");
    let expected_order = [
        "\"id\":",
        "\"id_hash\":",
        "\"kind\":",
        "\"site\":{\"word\":",
        "\"occurrence\":",
        "\"span\":{\"line\":",
        "\"col\":",
        "\"formula\":{\"op\":\"InRange\",\"value\":",
        "\"assumptions\":",
        "\"provenance\":",
    ];
    let mut pos = 0usize;
    for needle in expected_order {
        let idx = record[pos..].find(needle).unwrap_or_else(|| {
            panic!("obligation key `{needle}` missing or out of order in: {record}")
        });
        pos += idx + needle.len();
    }
}