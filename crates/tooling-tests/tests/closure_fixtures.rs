//! Cross-module assumption-closure fixtures (PLAN-VERIFY-3 P8.2): the
//! A-calls-B matrix executed on REAL langc artifacts through the T-CL image
//! walker (`tyu::closure::check_image_closure`).
//!
//! The two-module §11.1 shape: `Bank` (callee) contracts `withdraw` on the
//! named predicate `pct-in-range`; `App` imports and calls it, so App's
//! artifact carries a caller-side `contract-pre` obligation whose
//! `assumptions` edge names `Bank::withdraw::contract-pre::0` (the "B
//! proven ⇒ A dischargeable" matrix runs on exactly this edge).
//!
//! Matrix:
//!   - B proven ⇒ A dischargeable — both obligations closed, closure sound;
//!   - B unproven ⇒ A open — App's discharge rests on an unproven callee,
//!     so the walker forces it open with witness `assumption-unresolved`;
//!   - runtime-check terminal — the degraded extraction (callee artifact
//!     absent) carries the conservative `runtime-check` edge, the T-CL base
//!     case (the emitted check IS the discharge);
//!   - spurious edge — an edge naming an obligation nowhere in the image is
//!     unresolved (the conservative direction: more open, never a wrong
//!     discharge);
//!   - a cycle is E6419-malformed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

use tyu::closure::{check_image_closure, unresolved_witness, ImageClosure};
use verifier::testutil::{mk_echo, APP_MOD, BANK_DEF, BANK_MOD, TRIPLE};

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_closure_fixtures")
        .join(format!(
            "{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn compile_obl(dir: &Path, module: &str, source: &str) -> verifier::model::OblSet {
    fs::write(dir.join(format!("{module}.mod")), source).unwrap();
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--target={TRIPLE}"))
        .arg(format!("--out-dir={}", dir.display()))
        .arg(dir.join(format!("{module}.mod")).to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{module} must compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = fs::read(dir.join(format!("{module}.obl.json"))).unwrap();
    verifier::codec::read_obl(&bytes).expect("artifact must round-trip")
}

fn ob_pair(set: &verifier::model::OblSet, kind: verifier::model::Kind) -> (String, String) {
    let o = set
        .obligations
        .iter()
        .find(|o| o.kind == kind)
        .unwrap_or_else(|| panic!("{} must carry a {kind:?} obligation", set.module));
    (o.id.clone(), o.id_hash.clone())
}

/// The two-module scenario (edge-present): returns (bank_set, app_set, the
/// caller's contract-pre `(id, id_hash)`, the callee's `(id, id_hash)`).
fn two_module_sets(
    dir: &Path,
) -> (
    verifier::model::OblSet,
    verifier::model::OblSet,
    (String, String),
    (String, String),
) {
    let bank = compile_obl(dir, "Bank", BANK_MOD);
    fs::write(dir.join("Bank.def"), BANK_DEF).unwrap();
    let app = compile_obl(dir, "App", APP_MOD);
    let caller_pre = ob_pair(&app, verifier::model::Kind::ContractPre);
    let callee_pre = ob_pair(&bank, verifier::model::Kind::ContractPre);
    (bank, app, caller_pre, callee_pre)
}

fn run(
    modules: &[(
        String,
        Option<verifier::model::OblSet>,
        Option<verifier::verdict::Echo>,
    )],
) -> Result<ImageClosure, tyu::closure::ClosureCycle> {
    check_image_closure(modules)
}

/// B proven ⇒ A dischargeable: the callee's own `contract-pre` closed, the
/// caller's certificate rests on it — the image is well-closed.
#[test]
fn b_proven_makes_a_dischargeable() {
    common::ensure_bins();
    let dir = fresh_dir("b_proven");
    let (bank, app, caller_pre, callee_pre) = two_module_sets(&dir);
    let closure = run(&[
        (
            "Bank".to_string(),
            Some(bank),
            Some(mk_echo(&[(&callee_pre.0, &callee_pre.1)])),
        ),
        (
            "App".to_string(),
            Some(app),
            Some(mk_echo(&[(&caller_pre.0, &caller_pre.1)])),
        ),
    ])
    .expect("no cycle");
    assert!(closure.well_closed, "callee proven ⇒ caller closure sound");
    assert_eq!(closure.unresolved.len(), 0);
    // Both edge-bearing contract-pre obligations are walked: App's named
    // edge and Bank's own needs obligation (its self-transclusion degrades
    // to the runtime-check terminal while its artifact is still being
    // written — the T-CL base case).
    assert_eq!(closure.checked, 2);
}

/// B unproven ⇒ A open with witness: the caller's certificate rests on a
/// callee whose own `contract-pre` is open — the walker forces the caller
/// open with `assumption-unresolved: Bank::withdraw::contract-pre::0`.
#[test]
fn b_unproven_opens_a_with_witness() {
    common::ensure_bins();
    let dir = fresh_dir("b_unproven");
    let (bank, app, caller_pre, _callee_pre) = two_module_sets(&dir);
    // Bank's own contract-pre is NOT closed (no record in its echo).
    let closure = run(&[
        ("Bank".to_string(), Some(bank), Some(mk_echo(&[]))),
        (
            "App".to_string(),
            Some(app),
            Some(mk_echo(&[(&caller_pre.0, &caller_pre.1)])),
        ),
    ])
    .expect("no cycle");
    assert!(!closure.well_closed);
    assert_eq!(closure.unresolved.len(), 1);
    let u = &closure.unresolved[0];
    assert_eq!(u.module, "App");
    assert_eq!(u.dependency, "Bank::withdraw::contract-pre::0");
    assert_eq!(
        unresolved_witness(&u.dependency),
        "assumption-unresolved: Bank::withdraw::contract-pre::0"
    );
}

/// The degraded extraction (callee artifact unavailable) degrades the edge to
/// `runtime-check` — the T-CL base case — so a closed dependent is sound on
/// its own (the emitted check IS the discharge).
#[test]
fn runtime_check_edge_is_the_discharge() {
    common::ensure_bins();
    let dir = fresh_dir("runtime");
    // Only Bank.def — no Bank.obl.json (the callee was never extracted).
    fs::write(dir.join("Bank.def"), BANK_DEF).unwrap();
    let app = compile_obl(&dir, "App", APP_MOD);
    let caller_pre = ob_pair(&app, verifier::model::Kind::ContractPre);
    // The degraded artifact's edge is the runtime-check terminal.
    let pre = app
        .obligations
        .iter()
        .find(|o| o.id == caller_pre.0)
        .unwrap();
    assert!(
        pre.assumptions
            .iter()
            .any(|e| matches!(e, verifier::model::AssumptionEdge::RuntimeCheck)),
        "degraded extraction must carry the runtime-check edge: {:?}",
        pre.assumptions
    );
    let closure = run(&[(
        "App".to_string(),
        Some(app),
        Some(mk_echo(&[(&caller_pre.0, &caller_pre.1)])),
    )])
    .expect("no cycle");
    assert!(
        closure.well_closed,
        "runtime-check is the discharge (T-CL base case)"
    );
}

/// A spurious edge (naming an obligation nowhere in the image) is the
/// conservative direction: the dependent stays open — never a wrong
/// discharge.
#[test]
fn spurious_edge_keeps_dependent_open() {
    common::ensure_bins();
    let dir = fresh_dir("spurious");
    let (_bank, app, caller_pre, _callee_pre) = two_module_sets(&dir);
    // Tamper the caller's artifact: replace its edge target with a phantom.
    let mut app_t = app.clone();
    for o in &mut app_t.obligations {
        o.assumptions = vec![verifier::model::AssumptionEdge::Obligation {
            id: "Ghost::ghost-word::contract-pre::0".to_string(),
            module: "Ghost".to_string(),
        }];
    }
    let closure = run(&[(
        "App".to_string(),
        Some(app_t),
        Some(mk_echo(&[(&caller_pre.0, &caller_pre.1)])),
    )])
    .expect("no cycle");
    assert!(!closure.well_closed);
    assert_eq!(closure.unresolved.len(), 1);
    assert_eq!(
        closure.unresolved[0].dependency,
        "Ghost::ghost-word::contract-pre::0"
    );
}
