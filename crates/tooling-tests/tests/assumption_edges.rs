//! Cross-module assumption edges (PLAN-VERIFY-3 §Q7 rule 1, P1.2).
//!
//! A caller-side `contract-pre` obligation's discharge rests on the callee
//! module's own `needs` obligation. The edge is emitted at the transclusion
//! point: when the callee's artifact is available the edge names the callee
//! obligation by its canonical id; when it is not, the conservative
//! `runtime-check` edge holds (the emitted check IS the discharge — the
//! safe-failing direction of §Q7 rule 1).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use verifier::codec::read_obl;
use verifier::model::{AssumptionEdge, Kind};

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_assumption_edges")
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

/// The callee: `withdraw` contracts on the named predicate `pct-in-range`.
const BANK_MOD: &str = "\
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

/// The caller: imports `withdraw` and calls it — producing a caller-side
/// `contract-pre` obligation whose transclusion emits the assumption edge.
const APP_MOD: &str = "\
module App;
subtype Percent = i64 range 0..100;
import Bank { withdraw };

: main ( -- i64 )
  50 as Percent withdraw drop 0 ;

end;
";

/// The interface file (Q7: names only).
const BANK_DEF: &str = "\
module Bank;
subtype Percent = i64 range 0..100;

: pct-in-range ( Percent -- Percent bool ) ;
: withdraw ( Percent -- bool )
  needs [ pct-in-range ] ;
export { pct-in-range, withdraw } ;
end;
";

fn compile_obl(dir: &Path, module: &str, source: &str) -> Result<verifier::model::OblSet, String> {
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

fn caller_side_pre(set: &verifier::model::OblSet) -> &verifier::model::Obligation {
    set.obligations
        .iter()
        .find(|o| {
            o.kind == Kind::ContractPre
                && matches!(
                    o.formula,
                    verifier::model::Formula::PredicateHolds { ref pred, .. }
                        if pred.module == "Bank"
                )
        })
        .unwrap_or_else(|| panic!("App must carry a caller-side contract-pre for Bank"))
}

/// Edge-present fixture: the callee's artifact is on the search path, so the
/// caller-side `contract-pre` obligation gains an `Obligation` edge naming the
/// callee's own needs obligation by canonical id.
#[test]
fn callee_artifact_present_emits_named_edge() {
    let dir = fresh_dir("present");
    compile_obl(&dir, "Bank", BANK_MOD).expect("Bank must compile");
    std::fs::write(dir.join("Bank.def"), BANK_DEF).unwrap();
    let app = compile_obl(&dir, "App", APP_MOD).expect("App must compile");

    let pre = caller_side_pre(&app);
    assert!(
        pre.assumptions.iter().any(|e| matches!(
            e,
            AssumptionEdge::Obligation { id, module }
                if id == "Bank::withdraw::contract-pre::0" && module == "Bank"
        )),
        "the caller's contract-pre must edge to Bank::withdraw::contract-pre::0, got {:?}",
        pre.assumptions
    );
    // The transclusion also filled the formula and the trusted assumption.
    assert!(
        !matches!(pre.formula, verifier::model::Formula::PredicateHolds { ref pred, .. } if pred.ir.is_empty())
    );
}

/// Edge-degraded fixture (conservative direction): the callee's artifact is
/// absent (only the interface file is on the path) — the transclusion cannot
/// resolve the predicate facts, so the edge degrades to `runtime-check` and
/// the obligation stays open, never falsely discharged.
#[test]
fn callee_artifact_missing_degrades_to_runtime_check() {
    let dir = fresh_dir("absent");
    // Only Bank.def — no Bank.obl.json (the callee was never extracted).
    std::fs::write(dir.join("Bank.def"), BANK_DEF).unwrap();
    let app = compile_obl(&dir, "App", APP_MOD).expect("App must compile");

    let pre = caller_side_pre(&app);
    assert!(
        pre.assumptions
            .iter()
            .any(|e| matches!(e, AssumptionEdge::RuntimeCheck)),
        "an unavailable callee artifact must degrade to the runtime-check edge, got {:?}",
        pre.assumptions
    );
    // And it must NOT carry a fabricated named edge (conservative = safe).
    assert!(
        !pre.assumptions
            .iter()
            .any(|e| matches!(e, AssumptionEdge::Obligation { .. })),
        "no callee artifact ⇒ no named obligation edge (safe failing)"
    );
}

/// Spurious-edge safety: the conservative direction fails OPEN, never invents
/// a discharge. The closure check (P8) walks edges; a missing resolver state
/// surfaces the open obligation rather than a false proof.
#[test]
fn unresolved_callee_keeps_obligation_open_not_discharged() {
    // Same shape as the missing artifact: the record exists but there is no
    // discharge machinery — the VERDICT remains open (the runtime check is
    // retained). Assert on the artifact's own data: nonempty edges only in
    // the conservative shape, never a resolution signal.
    let dir = fresh_dir("open");
    let _ = std::fs::write(dir.join("Bank.def"), BANK_DEF);
    let app = compile_obl(&dir, "App", APP_MOD).expect("App must compile");
    for o in &app.obligations {
        if o.kind == Kind::ContractPre
            && matches!(o.formula, verifier::model::Formula::PredicateHolds { ref pred, .. } if pred.module == "Bank")
        {
            assert_eq!(
                o.assumptions,
                vec![AssumptionEdge::RuntimeCheck],
                "an unresolved callee must degrade to exactly one runtime-check edge"
            );
        }
    }
}
