//! Statement canonical-encoder tests (PLAN-VERIFY-3 P1.1).
//!
//! `verifier::stmt` is the hash the whole statement pipeline binds against,
//! so it lands before any other surface changes. Tests:
//!
//! - `determinism`: two runs → byte-identical canonical bytes and hash;
//! - `totality_over_corpus`: encode every obligation of a representative
//!   corpus (all kinds, every formula head) with no panic — the encoder is a
//!   total function `(Obligation, context) → canonical bytes` (FR-2). The
//!   real corpus totality (every `tooling-tests` fixture through langc) is
//!   exercised by `tooling-tests/tests/statement_goldens.rs` (P1.3), which
//!   depends on `langc` and can therefore run the full fixtures;
//! - `context_sensitivity`: changing any single context field changes the
//!   hash (§Q3 relativation: a statement proven on one target says nothing
//!   about another);
//! - `key_order_insensitivity`: the writer, not the caller, owns key order —
//!   equivalent statements assembled through different construction paths
//!   collapse to identical bytes, and the bytes match a hand-written expected
//!   canonical string;
//! - `known_answer`: one golden hash vector — catches accidental hash-
//!   algorithm drift.

use verifier::model::{ExtractionCtx, Formula, Kind, Oel, Provenance};
use verifier::stmt::{sha256_hex16, StatementContext, STMT_SCHEMA};

fn ctx_for(kind: Kind) -> StatementContext {
    StatementContext {
        semantics: verifier::semantics::SEMANTICS_VERSION.to_string(),
        stmt_ver: STMT_SCHEMA.to_string(),
        target: "x86_64-unknown-none".to_string(),
        model_semantics: "tyu.model/x86_64-unknown-none/1".to_string(),
        module: "Bank".to_string(),
        word: "withdraw".to_string(),
        word_ir_hash: "0123456789abcdef".to_string(),
        kind,
        occurrence: 0,
        refinement: None,
    }
}

#[test]
fn determinism_hash_twice_is_byte_equal() {
    let c = ctx_for(Kind::SubtypeRange);
    let formula = Formula::InRange {
        value: Oel::Var {
            name: "in.0".to_string(),
        },
        lo: 0,
        hi: 100,
    };
    let a = c.canonical_bytes(&formula);
    let b = c.canonical_bytes(&formula);
    assert_eq!(a, b, "canonical bytes must be byte-identical across runs");
    assert_eq!(c.statement_hash(&formula), c.statement_hash(&formula));
    assert_eq!(
        c.statement_hash_hex(&formula),
        c.statement_hash_hex(&formula)
    );
}

#[test]
fn totality_over_representative_corpus() {
    // One obligation per kind × formula head — the encoder must be total:
    // no panic, no unwrap on any context field, for every shape the
    // extractor can hand it.
    let mut ctx = ExtractionCtx::new(b"Bank");
    ctx.begin_word(b"clamp");
    let mut recorded = Vec::new();
    let id = ctx.record(
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
        1,
        2,
        Provenance::Opaque,
    );
    recorded.push(id);
    let id = ctx.record(
        Kind::ContractPre,
        Formula::PredicateHolds {
            pred: verifier::model::PredicateRef {
                module: "Math".to_string(),
                name: "nonneg".to_string(),
                ir: vec![
                    "block b0".to_string(),
                    "dup i64".to_string(),
                    "ret".to_string(),
                ],
                ir_hash: "11aa22bb".to_string(),
            },
            args: vec![Oel::Var {
                name: "$top".to_string(),
            }],
        },
        3,
        4,
        Provenance::Opaque,
    );
    recorded.push(id);
    ctx.begin_word(b"reg_read");
    let id = ctx.record(
        Kind::MmioBounds,
        Formula::OffsetLE {
            off: Some(0x1000),
            width: 4,
            size: 65536,
        },
        5,
        6,
        Provenance::Direct,
    );
    recorded.push(id);
    assert_eq!(recorded.len(), 3);

    let set = ctx.into_set();
    for o in &set.obligations {
        let c = StatementContext::for_obligation(
            "Bank",
            "x86_64-unknown-none",
            "unmodeled",
            "cafebabecafebabe",
            o,
        );
        // Total: must not panic on any context field and must return 32 bytes.
        let hash = c.statement_hash(&o.formula);
        assert_eq!(hash.len(), 32);
        assert_eq!(c.statement_hash_hex(&o.formula).len(), 64);
        assert!(!c.canonical_bytes(&o.formula).is_empty());
    }
}

#[test]
fn context_sensitivity_each_field_varies_the_hash() {
    let base_formula = Formula::InRange {
        value: Oel::Var {
            name: "in.0".to_string(),
        },
        lo: 0,
        hi: 100,
    };
    let base = ctx_for(Kind::SubtypeRange);
    let base_hash = base.statement_hash(&base_formula);

    // Every relativity/identity field, varied independently, must change the
    // hash — a statement proven on one (target, model) says nothing about
    // another (§Q3).
    let mut variants: Vec<StatementContext> = Vec::new();
    let mut other = base.clone();
    other.target = "armv7m-unknown-none".to_string();
    variants.push(other);
    let mut other = base.clone();
    other.model_semantics = "tyu.model/armv7m-unknown-none/1".to_string();
    variants.push(other);
    let mut other = base.clone();
    other.word_ir_hash = "ffffffffffffffff".to_string();
    variants.push(other);
    let mut other = base.clone();
    other.module = "Other".to_string();
    variants.push(other);
    let mut other = base.clone();
    other.word = "deposit".to_string();
    variants.push(other);
    let mut other = base.clone();
    other.occurrence = 1;
    variants.push(other);
    let mut other = base.clone();
    other.kind = Kind::ContractPost;
    variants.push(other);
    let mut other = base.clone();
    other.semantics = "tyu.ir-sem/999.0".to_string();
    variants.push(other);
    let mut other = base.clone();
    other.refinement = Some("rp2350.uart".to_string());
    variants.push(other);

    for v in &variants {
        assert_ne!(
            v.statement_hash(&base_formula),
            base_hash,
            "changing a context field must change statement_hash"
        );
    }

    // Same context, different formula → different hash.
    let other_formula = Formula::InRange {
        value: Oel::Var {
            name: "out.0".to_string(),
        },
        lo: 0,
        hi: 100,
    };
    assert_ne!(
        base.statement_hash(&other_formula),
        base_hash,
        "formula change must change statement_hash"
    );
}

#[test]
fn key_order_insensitivity_writer_owns_ordering() {
    // The same statement assembled through different construction paths
    // (context built by hand vs built by the record-driven builder) must
    // collapse to identical bytes, and the bytes must equal the hand-written
    // canonical form — proving the *writer* owns key order, never the caller.
    let formula = Formula::InRange {
        value: Oel::Var {
            name: "in.0".to_string(),
        },
        lo: 0,
        hi: 100,
    };
    let hand = ctx_for(Kind::SubtypeRange);

    // Builder path: same module/word/kind/occurrence via `for_obligation`.
    let mut record_ctx = ExtractionCtx::new(b"Bank");
    record_ctx.begin_word(b"withdraw");
    let (id, _) = record_ctx.record(
        Kind::SubtypeRange,
        formula.clone(),
        0,
        0,
        Provenance::Direct,
    );
    assert!(id.starts_with("Bank::withdraw::subtype-range::0"));
    let set = record_ctx.into_set();
    let built = StatementContext::for_obligation(
        "Bank",
        "x86_64-unknown-none",
        "tyu.model/x86_64-unknown-none/1",
        "0123456789abcdef",
        &set.obligations[0],
    );
    assert_eq!(built, hand, "the two construction paths must agree");

    // Canonical bytes = fixed, hand-checkable form (keys lexicographic).
    let bytes = hand.canonical_bytes(&formula);
    let text = String::from_utf8(bytes).expect("ascii");
    let expected = concat!(
        r#"{"context":{"kind":"subtype-range","model_semantics":"tyu.model/x86_64-unknown-none/1","#,
        r#""module":"Bank","occurrence":0,"refinement":null,"semantics":"tyu.ir-sem/1.0","#,
        r#""stmt_ver":"tyu.stmt/1.0","target":"x86_64-unknown-none","word":"withdraw","#,
        r#""word_ir_hash":"0123456789abcdef"},"formula":{"hi":100,"lo":0,"op":"InRange","#,
        r#""value":{"name":"in.0","op":"Var"}}}"#,
    );
    assert_eq!(
        text, expected,
        "canonical key order is fixed (lexicographic)"
    );
}

#[test]
fn known_answer_golden_hash_vector() {
    // A golden hash vector: changing the SHA-256 wiring or the canonical
    // encoding changes this constant, failing the golden gate (P1.3). Freeze
    // the whole canonical text, not just the hash, so a change is diagnosed
    // at the encoding level.
    let c = ctx_for(Kind::SubtypeRange);
    let formula = Formula::InRange {
        value: Oel::Var {
            name: "in.0".to_string(),
        },
        lo: 0,
        hi: 100,
    };
    // Freeze the canonical text the hash covers (guards against a hash that
    // silently stops covering the statement).
    let text = String::from_utf8(c.canonical_bytes(&formula)).unwrap();
    assert_eq!(
        text,
        concat!(
            r#"{"context":{"kind":"subtype-range","model_semantics":"tyu.model/x86_64-unknown-none/1","#,
            r#""module":"Bank","occurrence":0,"refinement":null,"semantics":"tyu.ir-sem/1.0","#,
            r#""stmt_ver":"tyu.stmt/1.0","target":"x86_64-unknown-none","word":"withdraw","#,
            r#""word_ir_hash":"0123456789abcdef"},"formula":{"hi":100,"lo":0,"op":"InRange","#,
            r#""value":{"name":"in.0","op":"Var"}}}"#,
        )
    );
    // Golden hash (catch accidental algorithm drift). The expected value is
    // pinned by a committed run; change ⇨ the golden gate fails.
    assert_eq!(
        c.statement_hash_hex(&formula),
        "5f7401281e3e0a9d96b57de576ad3d485498cfef4755129419723b3811fc9001",
        "statement_hash golden drifted — see tooling-tests statement goldens"
    );
}

#[test]
fn sha256_hex16_is_16_lowercase_digits() {
    // FR-14: word_ir_hash is the 16-hex-char SHA-256 prefix form.
    let h = sha256_hex16(b"local_get 0\nconst_i64 100\nle_bool");
    assert_eq!(h.len(), 16);
    assert!(h
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(sha256_hex16(b"x"), sha256_hex16(b"x"), "deterministic");
    assert_ne!(sha256_hex16(b"x"), sha256_hex16(b"y"));
}
