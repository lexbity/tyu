//! Cross-module contract discharge plumbing (static-verification.md Q6/Q7,
//! slice P6).
//!
//! The `.def` boundary carries contract clause *names only* (§6.6 — no
//! bodies, no `bound`); the compiler-computed predicate IR travels in the
//! callee module's `.obl.json` `facts.predicates`. The caller's build
//! transcludes the callee's predicate into its `contract-pre` obligations,
//! and a stale callee artifact — a `.def` naming a predicate the callee's
//! artifact no longer documents — fails loudly with E6413 (never a silent
//! open).
//!
//! Two modules, one directory (the `try_load_module_file` search base):
//!
//! - `Bank` declares `Percent`, the pure predicate `pct-in-range`, and the
//!   contracted word `withdraw` (`needs [ pct-in-range ]`);
//! - `App` imports `withdraw` and calls it, so App's artifact carries a
//!   caller-side `contract-pre` obligation that transcludes `pct-in-range`.
//!
//! The staleness scenario recompiles `Bank` against a *drifted* predicate
//! (`needs [ pct-ok ]`, exports kept aligned) while the hand-authored
//! `Bank.def` still names `pct-in-range`: the caller's transclusion finds no
//! matching fact and the build fails E6413 (R6 detected, fail-loud).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use verifier::codec::read_obl;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_contract_crossmod")
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

/// The callee module, v1: `withdraw` contracts on `pct-in-range`. The named
/// predicate mirrors the inline idiom: it leaves the input below the verdict
/// (`-- Percent bool`), exactly as the book's `dup 0 >= [ … ] if` shape does.
const BANK_MOD_V1: &str = "\
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

/// The callee module, v2 (drifted): the contract now names `pct-ok`; the
/// export surface is kept aligned with `BANK_DEF` so the import machinery
/// passes and the *transclusion* is what finds the stale interface.
const BANK_MOD_V2: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
export { pct-in-range, withdraw } ;

: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: pct-ok ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: withdraw ( Percent -- bool )
  needs [ pct-ok ]
  drop true ;

end;
";

/// The hand-authored boundary file (Q7): names only — the contract clause
/// is a predicate name, never a body, never a bound.
const BANK_DEF: &str = "\
module Bank;
subtype Percent = i64 range 0..100;

: pct-in-range ( Percent -- Percent bool ) ;
: withdraw ( Percent -- bool )
  needs [ pct-in-range ] ;
export { pct-in-range, withdraw } ;
end;
";

/// The caller: imports `withdraw` and calls it once.
const APP_MOD: &str = "\
module App;
subtype Percent = i64 range 0..100;
import Bank { withdraw };

: main ( -- i64 )
  50 as Percent withdraw drop 0 ;

end;
";

fn compile_obl_in(dir: &Path, source: &str, module: &str) -> Result<verifier::model::OblSet, String> {
    let mod_path = dir.join(format!("{module}.mod"));
    std::fs::write(&mod_path, source).unwrap();
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();
    if out.status.success() {
        let bytes = std::fs::read(dir.join(format!("{module}.obl.json"))).unwrap();
        Ok(read_obl(&bytes).expect("artifact must round-trip"))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[test]
fn callee_artifact_carries_predicate_facts_and_definition_site() {
    let dir = fresh_dir("callee");
    let set = compile_obl_in(&dir, BANK_MOD_V1, "Bank").expect("Bank must compile");

    // `facts.predicates` transcribes the named predicate's op-text IR.
    let pred = set
        .facts
        .predicates
        .iter()
        .find(|p| p.name == "pct-in-range")
        .expect("Bank facts must carry pct-in-range");
    assert!(!pred.ir.is_empty(), "predicate IR must be transcluded");
    assert!(!pred.ir_hash.is_empty());

    // The callee-side `contract-pre` definition-site record: `withdraw`'s
    // own `needs` clause.
    let pre = set
        .obligations
        .iter()
        .find(|o| {
            o.kind == verifier::model::Kind::ContractPre && o.site.word == "withdraw"
        })
        .expect("withdraw must carry a callee-side contract-pre record");
    match &pre.formula {
        verifier::model::Formula::PredicateHolds { pred, .. } => {
            assert_eq!(pred.name, "pct-in-range");
        }
        other => panic!("contract-pre must be PredicateHolds, got {other:?}"),
    }
}

#[test]
fn caller_transcludes_the_callee_predicate() {
    let dir = fresh_dir("caller");
    compile_obl_in(&dir, BANK_MOD_V1, "Bank").expect("Bank must compile");
    let _ = std::fs::write(dir.join("Bank.def"), BANK_DEF);

    let app = compile_obl_in(&dir, APP_MOD, "App").expect("App must compile");

    // App::main's call to `withdraw` yields a caller-side contract-pre record
    // whose transcluded predicate resolves to Bank's `pct-in-range` with its
    // compiler-computed IR + hash (Q7: self-contained, hash-checked).
    let pre = app
        .obligations
        .iter()
        .find(|o| o.kind == verifier::model::Kind::ContractPre && o.site.word == "main")
        .expect("App::main must carry a caller-side contract-pre record");
    match &pre.formula {
        verifier::model::Formula::PredicateHolds { pred, .. } => {
            assert_eq!(pred.module, "Bank", "predicate must resolve to its callee module");
            assert_eq!(pred.name, "pct-in-range");
            assert!(!pred.ir.is_empty(), "transclusion must carry the predicate IR");
            assert!(!pred.ir_hash.is_empty(), "transclusion must carry the predicate hash");
        }
        other => panic!("contract-pre must be PredicateHolds, got {other:?}"),
    }
    // The record lists the transcluded predicate as a trusted assumption (T2).
    assert!(
        pre.assumptions.iter().any(|a| matches!(
            a,
            verifier::model::Assumption::ContractPredicate { name, .. }
                if name == "pct-in-range"
        )),
        "transclusion must list the predicate as a trusted assumption"
    );
}

#[test]
fn stale_callee_artifact_fails_loudly_with_e6413() {
    let dir = fresh_dir("stale");
    compile_obl_in(&dir, BANK_MOD_V1, "Bank").expect("Bank v1 must compile");
    let _ = std::fs::write(dir.join("Bank.def"), BANK_DEF);

    // Recompile Bank against the drifted predicate (v2), leaving the
    // interface file stale.
    compile_obl_in(&dir, BANK_MOD_V2, "Bank").expect("Bank v2 must compile");

    // The caller's transclusion finds no `pct-in-range` fact in Bank's new
    // artifact → E6413, never a silent open (R6 fail-loud).
    let mod_path = dir.join("App.mod");
    std::fs::write(&mod_path, APP_MOD).unwrap();
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("[E6413]"),
        "stale callee interface must surface E6413, stderr was: {stderr}"
    );
}