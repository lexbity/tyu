//! The `--obl` artifact-path agreement (PLAN-VERIFY-3 P14.2, review findings
//! 1/4): the port's `rederive` exe and the in-tree `discharge_word` must
//! classify EVERY obligation of a real `tyu.obl/v2` artifact the same way —
//! and the analysis context (per-cast subtype ranges, word width) must be
//! threaded, never hardcoded.
//!
//! The `Small` artifact is the soundness regression the fix exists for:
//! `50 as Small ([-10,10]) as Big ([0,1000])`. With the WRONG cast range
//! (a hardcoded `0..=100`) the abstract value survives the first cast and
//! the SECOND cast site discharges — a `proof` for code that actually traps
//! at the first cast. Both engines must leave it open.
//!
//! Also pinned: fail-loud behavior (a missing word / unparseable blocks fail
//! the module, never a silent drop).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

mod common;

use ir::{Block, BlockId, EffectSet, Op, OpKind, Sig, Span, TypeId};
use verifier::interp::{discharge_word, FlatMem, SubtypeRange};
use verifier::model::{Cycle, Formula, Intent, Kind, Obligation, Oel, Provenance, Site, SpanInfo};
use verifier::target::TargetSpec;

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn rederive_exe() -> PathBuf {
    workspace_root().join("verification/ports/lean/.lake/build/bin/rederive")
}

fn have_rederive() -> bool {
    rederive_exe().exists()
}

/// The multi-subtype artifact: two casts to DIFFERENT bands. The first cast
/// (50 → Small) is provably-failing; the value after it is empty, so the
/// second cast site (→ Big) is UNREACHABLE — it must stay open.
const SMALL_ARTIFACT: &str = r#"{
  "schema": "tyu.obl/v2", "semantics": "tyu.ir-sem/1.0", "stmt": "tyu.stmt/1.0",
  "module": "Small", "target": "x86_64-unknown-none", "platform": "x86_64-unknown-none",
  "model_semantics": "unmodeled", "abi_contract_version": 2,
  "facts": {
    "words": [ { "name": "f", "net": 1, "high": 1, "top": false,
      "diverge_free": true, "blocks": 1,
      "ir": "block b0\nconst_i64 50\ncast Small\ncast Big\nret",
      "ir_hash": "aaaaaaaaaaaaaaaa" } ],
    "subtypes": [ { "name": "Small", "lo": -10, "hi": 10 },
                  { "name": "Big", "lo": 0, "hi": 1000 } ],
    "predicates": [] },
  "obligations": [
    { "id": "Small::f::subtype-range::0", "id_hash": "1111111111111111",
      "kind": "subtype-range",
      "site": { "word": "f", "occurrence": 0, "span": { "line": 1, "col": 1 } },
      "formula": { "op": "InRange", "lo": -10, "hi": 10,
                   "value": { "op": "Cast", "from": "i64", "to": "Small",
                              "arg": { "op": "Var", "name": "$top" } } },
      "assumptions": [], "cycles": [] },
    { "id": "Small::f::subtype-range::1", "id_hash": "2222222222222222",
      "kind": "subtype-range",
      "site": { "word": "f", "occurrence": 1, "span": { "line": 1, "col": 2 } },
      "formula": { "op": "InRange", "lo": 0, "hi": 1000,
                   "value": { "op": "Cast", "from": "Small", "to": "Big",
                              "arg": { "op": "Var", "name": "$top" } } },
      "assumptions": [], "cycles": [] }
  ]
}"#;

/// The in-tree discharge of the artifact's word. The subtype range lookup
/// mirrors the artifact's `facts.subtypes` (Small → [-10,10], Big →
/// [0,1000]); everything else None — the REAL analysis context finding 1
/// requires the rederive to reproduce.
fn sr() -> &'static SubtypeRange<'static> {
    &|tid: TypeId| match tid {
        TypeId(3) => Some((-10, 10)),
        TypeId(4) => Some((0, 1000)),
        _ => None,
    }
}

fn obligation(
    id: &str,
    id_hash: &str,
    occurrence: u32,
    from: &str,
    to: &str,
    lo: i64,
    hi: i64,
) -> Obligation {
    Obligation {
        id: id.to_string(),
        id_hash: id_hash.to_string(),
        kind: Kind::SubtypeRange,
        site: Site {
            word: "f".to_string(),
            occurrence,
            span: SpanInfo { line: 0, col: 0 },
        },
        intent: Intent {
            label: String::new(),
            subject: String::new(),
            authored: false,
        },
        formula: Formula::InRange {
            value: Oel::Cast {
                from: from.to_string(),
                to: to.to_string(),
                // The cast sites' provenance is opaque (`$top`).
                arg: Box::new(Oel::Var {
                    name: "$top".to_string(),
                }),
            },
            lo,
            hi,
        },
        assumptions: Vec::new(),
        cycles: Vec::<Cycle>::new(),
        provenance: Provenance::Opaque,
    }
}

fn build_word() -> ir::Word {
    let mut w = ir::Word {
        name: ir::Atom::new(b"f").unwrap(),
        sig: Sig {
            in_len: 0,
            out_len: 1,
            ..Sig::empty()
        },
        performs: EffectSet::empty(),
        requires: ir::CapSet::empty(),
        bound: ir::StackBound::ID,
        entry: BlockId(0),
        types: Default::default(),
        type_sizes: Default::default(),
        type_classes: Default::default(),
        apertures: Default::default(),
        subtype_bases: Default::default(),
        blocks: Default::default(),
    };
    w.types.push(ir::Atom::new(b"i64").unwrap()).unwrap();
    w.types.push(ir::Atom::new(b"bool").unwrap()).unwrap();
    w.types.push(ir::Atom::new(b"percent").unwrap()).unwrap();
    w.types.push(ir::Atom::new(b"Small").unwrap()).unwrap();
    w.types.push(ir::Atom::new(b"Big").unwrap()).unwrap();
    let ops = [
        OpKind::ConstI64(50),
        OpKind::Cast {
            from: TypeId(0),
            to: TypeId(3),
        },
        OpKind::Cast {
            from: TypeId(0),
            to: TypeId(4),
        },
        OpKind::Ret,
    ];
    let block = {
        let mut b = Block {
            id: BlockId(0),
            entry_stack: Default::default(),
            ops: Default::default(),
        };
        for op in &ops {
            b.ops
                .push(Op {
                    kind: *op,
                    span: Span::UNKNOWN,
                })
                .unwrap();
        }
        b
    };
    w.blocks.push(block).unwrap();
    w
}

fn run_rederive(dir: &std::path::Path, artifact: &str, name: &str) -> (bool, String) {
    let path = dir.join(name);
    fs::write(&path, artifact).unwrap();
    let out = dir.join("rd.json");
    let s = Command::new(rederive_exe())
        .arg("--obl")
        .arg(&path)
        .arg("--out")
        .arg(&out)
        .arg("--toolchain")
        .arg("lean4:4.27.0+port")
        .output()
        .expect("run rederive --obl");
    let stdout = String::from_utf8_lossy(&s.stdout).to_string();
    (s.status.success(), stdout)
}

#[test]
fn artifact_discharge_agrees_with_in_tree_context() {
    if !have_rederive() {
        if std::env::var("TYU_REDERIVE_E2E").is_ok() {
            panic!(
                "TYU_REDERIVE_E2E requires the built rederive exe (ci/differential.sh builds it)"
            );
        }
        eprintln!("skipping the artifact agreement e2e (no rederive exe)");
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "tyu-rederive-agree={}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();

    // The in-tree classification (the obligations mirror the artifact's two
    // cast sites; the word is the artifact's `f` op-for-op).
    let word = build_word();
    let mut mem = FlatMem;
    let obligations = [
        obligation(
            "Small::f::subtype-range::0",
            "1111111111111111",
            0,
            "i64",
            "Small",
            -10,
            10,
        ),
        obligation(
            "Small::f::subtype-range::1",
            "2222222222222222",
            1,
            "Small",
            "Big",
            0,
            1000,
        ),
    ];
    let verdicts = discharge_word(&word, &obligations, sr(), TargetSpec::X86_64, &mut mem);
    assert_eq!(verdicts.len(), 2, "two obligations");
    assert_eq!(
        verdicts[0].status,
        verifier::verdict::VerdictStatus::Open,
        "cast 0 (50 → Small): provably-failing — check retained"
    );
    assert!(
        verdicts[0].provably_failing,
        "cast 0 is provably failing (out of [-10,10])"
    );
    assert_eq!(
        verdicts[1].status,
        verifier::verdict::VerdictStatus::Open,
        "cast 1 (→ Big): the flow after cast 0 is empty — MUST stay open"
    );

    // The port's rederive must classify identically.
    let (ok, logs) = run_rederive(&dir, SMALL_ARTIFACT, "Small.obl.json");
    assert!(ok, "rederive --obl failed:\n{logs}");
    let rd = fs::read_to_string(dir.join("rd.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&rd).expect("v2 doc");
    let records = doc["verdicts"].as_array().expect("verdicts");
    assert_eq!(records.len(), 2);
    for r in records {
        assert_eq!(
            r["status"], "open",
            "both cast sites must stay open with the artifact's real subtype \
             ranges (a discharge here = the wrong-range false positive): {}",
            rd
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

/// Fail-loud: an obligation referencing a word missing from `facts.words`
/// is a HARD failure — never a silent drop that reads as open-by-absence.
#[test]
fn missing_word_fails_loudly() {
    if !have_rederive() {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "tyu-rederive-fail={}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let art = SMALL_ARTIFACT
        .replace(
            "\"words\": [ { \"name\": \"f\",",
            "\"words\": [ { \"name\": \"other\",",
        )
        .replace("block b0\n", "");
    let (ok, logs) = run_rederive(&dir, &art, "Missing.obl.json");
    assert!(
        !ok,
        "rederive must FAIL loudly on an obligation whose word is missing: {logs}"
    );
    assert!(
        logs.contains("missing from facts.words") || logs.contains("FAIL"),
        "the failure must name the missing word: {logs}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The positive control: a `percent 0..=100` artifact still discharges its
/// in-band casts via re-derivation (the proven_automation_only path).
#[test]
fn artifact_agreement_percent_still_discharges() {
    if !have_rederive() {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "tyu-rederive-perc={}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let (ok, logs) = run_rederive(&dir, PERCENT_ARTIFACT, "Perc.obl.json");
    assert!(ok, "rederive --obl failed:\n{logs}");
    let rd = fs::read_to_string(dir.join("rd.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&rd).expect("v2 doc");
    for r in doc["verdicts"].as_array().expect("verdicts") {
        assert_eq!(
            r["status"], "discharged",
            "in-band percent casts still discharge (positive control): {rd}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// A single-band `percent` artifact (the positive control).
const PERCENT_ARTIFACT: &str = r#"{
  "schema": "tyu.obl/v2", "semantics": "tyu.ir-sem/1.0", "stmt": "tyu.stmt/1.0",
  "module": "Perc", "target": "x86_64-unknown-none", "platform": "x86_64-unknown-none",
  "model_semantics": "unmodeled", "abi_contract_version": 2,
  "facts": {
    "words": [ { "name": "f", "net": 1, "high": 1, "top": false,
      "diverge_free": true, "blocks": 1,
      "ir": "block b0\nconst_i64 50\ncast Percent\nret",
      "ir_hash": "aaaaaaaaaaaaaaaa" } ],
    "subtypes": [ { "name": "Percent", "lo": 0, "hi": 100 } ],
    "predicates": [] },
  "obligations": [
    { "id": "Perc::f::subtype-range::0", "id_hash": "1111111111111111",
      "kind": "subtype-range",
      "site": { "word": "f", "occurrence": 0, "span": { "line": 1, "col": 1 } },
      "formula": { "op": "InRange", "lo": 0, "hi": 100,
                   "value": { "op": "Cast", "from": "i64", "to": "Percent",
                              "arg": { "op": "Var", "name": "$top" } } },
      "assumptions": [], "cycles": [] },
    { "id": "Perc::f::subtype-range::1", "id_hash": "2222222222222222",
      "kind": "subtype-range",
      "site": { "word": "f", "occurrence": 1, "span": { "line": 1, "col": 1 } },
      "formula": { "op": "InRange", "lo": 0, "hi": 100,
                   "value": { "op": "Var", "name": "out.0" } },
      "assumptions": [], "cycles": [] }
  ]
}"#;
