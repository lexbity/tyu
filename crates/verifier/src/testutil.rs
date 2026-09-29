//! Test-fixture builders for the verification data model (PLAN-VERIFY-3
//! P8.2): the `OblSet`/`Obligation`/`Echo`/`VerdictRecord` constructors the
//! closure tests (and the closure fixtures) all hand-rolled — one shared
//! home so a fourth copy never appears.
//!
//! Gated behind the `test-util` feature (enabled in the consuming crates'
//! dev-dependencies); `#[doc(hidden)]`: fixture support, not public API.

#![allow(dead_code)]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::model::{
    AssumptionEdge, Facts, Formula, Intent, Kind, OblSet, Obligation, Oel, Provenance, Site,
    SpanInfo,
};
use crate::verdict::{Echo, Trust, VerdictRecord, VerdictStatus, Verdicts};

/// The triple the cross-module fixtures are extracted for.
pub const TRIPLE: &str = "x86_64-unknown-linux-gnu";

/// The module-source fixtures (callee `Bank` + caller `App` + the
/// interface file) shared by the closure test surfaces.
pub const BANK_MOD: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
export { pct-in-range, withdraw } ;

: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: withdraw ( Percent -- bool )
  needs [ pct-in-range ]
  drop true ;

end;
";

pub const APP_MOD: &str = "\
module App;
subtype Percent = i64 range 0..100;
import Bank { withdraw };

: main ( -- i64 )
  50 as Percent withdraw drop 0 ;

end;
";

pub const BANK_DEF: &str = "\
module Bank;
subtype Percent = i64 range 0..100;

: pct-in-range ( Percent -- Percent bool ) ;
: withdraw ( Percent -- bool )
  needs [ pct-in-range ] ;
export { pct-in-range, withdraw } ;
end;
";

/// A named `contract-pre` assumption edge (`Obligation { id, module }`).
pub fn edge(module: &str, id: &str) -> AssumptionEdge {
    AssumptionEdge::Obligation {
        id: id.to_string(),
        module: module.to_string(),
    }
}

/// One obligation record with the canonical fixture shape: word `w`,
/// occurrence `0`, span (1,1), `InRange` formula over `in.0` `0..100`.
pub fn mk_obligation(
    id: &str,
    id_hash: &str,
    kind: Kind,
    assumptions: Vec<AssumptionEdge>,
) -> Obligation {
    Obligation {
        id: id.into(),
        id_hash: id_hash.into(),
        kind,
        site: Site {
            word: "w".into(),
            occurrence: 0,
            span: SpanInfo { line: 1, col: 1 },
        },
        intent: Intent {
            label: String::new(),
            subject: String::new(),
            authored: false,
        },
        formula: Formula::InRange {
            value: Oel::Var {
                name: "in.0".into(),
            },
            lo: 0,
            hi: 100,
        },
        assumptions,
        cycles: Vec::new(),
        provenance: Provenance::Direct,
    }
}

/// A single-obligation artifact with the fixed header identity (triple
/// `x86_64-unknown-linux-gnu`, `model_semantics: "unmodeled"`).
pub fn mk_obl_set(module: &str, obligations: Vec<Obligation>) -> OblSet {
    OblSet {
        schema: "tyu.obl/v2".into(),
        semantics: crate::semantics::SEMANTICS_VERSION.to_string(),
        stmt: crate::stmt::STMT_SCHEMA.to_string(),
        module: module.into(),
        target: TRIPLE.into(),
        platform: TRIPLE.into(),
        model_semantics: "unmodeled".into(),
        concurrency: "unmodeled".into(),
        abi_contract_version: 2,
        facts: Facts {
            words: Vec::new(),
            subtypes: Vec::new(),
            predicates: Vec::new(),
        },
        obligations,
    }
}

/// One `contract-pre` obligation in `module` with the given (module-internal)
/// dependency edges.
pub fn mk_contract_pre(module: &str, id: &str, id_hash: &str, deps: &[&str]) -> Obligation {
    mk_obligation(
        id,
        id_hash,
        Kind::ContractPre,
        deps.iter().map(|d| edge(module, d)).collect(),
    )
}

/// A synthetic multi-obligation artifact: `(id, id_hash, dep-ids)`.
pub fn mk_synthetic_set(module: &str, defs: &[(&str, &str, &[&str])]) -> OblSet {
    let obligations = defs
        .iter()
        .map(|(id, id_hash, deps)| mk_contract_pre(module, id, id_hash, deps))
        .collect();
    mk_obl_set(module, obligations)
}

/// A default discharged `proof`/`certificate` verdict record (the shaped the
/// harvest emits for a closed site).
pub fn mk_discharged_record(id: &str, id_hash: &str) -> VerdictRecord {
    VerdictRecord {
        id: id.to_string(),
        id_hash: id_hash.to_string(),
        status: VerdictStatus::Discharged,
        trust: Trust::Proof,
        method: Some(crate::verdict::Method::Certificate),
        surface: None,
        statement_hash: Some("f".repeat(64)),
        authored: None,
        proof: None,
        claimed: None,
        justification: None,
        witness_reason: None,
        note: None,
    }
}

/// A harvested-style echo document: the given `(id, id_hash)` pairs resolved
/// `discharged`/`proof`/`certificate` (a missing record means open — the
/// echo only carries closed records).
pub fn mk_echo(closed: &[(&str, &str)]) -> Echo {
    let records: Vec<VerdictRecord> = closed
        .iter()
        .map(|(id, id_hash)| mk_discharged_record(id, id_hash))
        .collect();
    Echo {
        verdicts: Verdicts {
            semantics: crate::semantics::SEMANTICS_VERSION.to_string(),
            stmt: crate::stmt::STMT_SCHEMA.to_string(),
            certifier: None,
            target: TRIPLE.into(),
            model_semantics: "unmodeled".into(),
            concurrency: "unmodeled".into(),
            records,
        },
        stale_verdicts: 0,
        emitted: Default::default(),
        provably_failing: Vec::new(),
        open_reasons: Vec::new(),
        file_verdicts: 0,
        in_tree_verdicts: 0,
    }
}

/// A harvested-style `tyu.verdicts/v2` document text with the given
/// closed records (the shape `tyu::closure::apply_harvest_closure`
/// consumes). The certifier is the recognized lean port.
pub fn mk_harvest_doc(closed: &[(&str, &str)]) -> String {
    let recs: Vec<String> = closed
        .iter()
        .map(|(id, id_hash)| {
            format!(
                "{{\"id\":\"{}\",\"id_hash\":\"{}\",\"status\":\"discharged\",\"trust\":\"proof\",\"method\":\"certificate\",\"statement_hash\":\"{}\",\"proof\":{{\"kind\":\"certificate\",\"statement\":\"tyu.stmt/1.0\"}}}}",
                id,
                id_hash,
                "f".repeat(64)
            )
        })
        .collect();
    format!(
        "{{\"schema\":\"tyu.verdicts/v2\",\"certifier\":{{\"class\":\"port\",\"name\":\"lean\",\"recognition\":\"tyu-port/lean/1\",\"tool\":{{\"name\":\"harvest\",\"version\":\"0.1.0\"}},\"toolchain\":\"lean4:4.27.0\"}},\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"target\":\"{}\",\"model_semantics\":\"unmodeled\",\"concurrency\":\"unmodeled\",\"verdicts\":[{}]}}",
        TRIPLE,
        recs.join(",")
    )
}
