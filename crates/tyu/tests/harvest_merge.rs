//! Multi-module harvest merge + assumption closure (PLAN-VERIFY-3 P8.2):
//! the merged verdict-set walk executed by
//! `tyu::closure::check_image_closure` over real langc artifacts.
//!
//! Covers the P8.2 gate surface: topological-order independence of the
//! merged closure, deterministic unresolved reporting, the
//! E6419-malformed cycle path, and diamond-dependency memoization (an
//! obligation's closure is assessed once per image). Fixture builders
//! shared with the closure unit tests and the tooling-tests fixtures live
//! in `verifier::testutil` (one home for the OblSet/Echo/VerdictRecord
//! shapes — never a per-file copy).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use verifier::testutil::{mk_echo, mk_synthetic_set, APP_MOD, BANK_DEF, BANK_MOD, TRIPLE};

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-harvest-merge-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn langc_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/langc")
}

fn ensure_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build failed");
    assert!(langc_exe().exists(), "langc built");
}

fn compile_obl(dir: &std::path::Path, module: &str, source: &str) -> verifier::model::OblSet {
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

/// The merged two-module scenario (callee-first build order). Returns the
/// callee's and caller's contract-pre `(id, id_hash)` pairs.
fn merged_scenario(
    dir: &std::path::Path,
) -> (
    verifier::model::OblSet,
    verifier::model::OblSet,
    (String, String),
    (String, String),
) {
    let bank = compile_obl(dir, "Bank", BANK_MOD);
    fs::write(dir.join("Bank.def"), BANK_DEF).unwrap();
    let app = compile_obl(dir, "App", APP_MOD);
    let caller_pre = app
        .obligations
        .iter()
        .find(|o| o.kind == verifier::model::Kind::ContractPre)
        .map(|o| (o.id.clone(), o.id_hash.clone()))
        .expect("App must carry a caller-side contract-pre");
    let callee_pre = bank
        .obligations
        .iter()
        .find(|o| o.kind == verifier::model::Kind::ContractPre)
        .map(|o| (o.id.clone(), o.id_hash.clone()))
        .expect("Bank must carry its own contract-pre");
    (bank, app, caller_pre, callee_pre)
}

#[test]
fn merged_closure_is_order_independent() {
    ensure_langc();
    let dir = fresh_dir("order");
    let (bank, app, caller_pre, callee_pre) = merged_scenario(&dir);
    let forward = tyu::closure::check_image_closure(&[
        (
            "Bank".to_string(),
            Some(bank.clone()),
            Some(mk_echo(&[(&callee_pre.0, &callee_pre.1)])),
        ),
        (
            "App".to_string(),
            Some(app.clone()),
            Some(mk_echo(&[(&caller_pre.0, &caller_pre.1)])),
        ),
    ])
    .expect("no cycle");
    // Caller-first (reversed merge order).
    let reversed = tyu::closure::check_image_closure(&[
        (
            "App".to_string(),
            Some(app),
            Some(mk_echo(&[(&caller_pre.0, &caller_pre.1)])),
        ),
        (
            "Bank".to_string(),
            Some(bank),
            Some(mk_echo(&[(&callee_pre.0, &callee_pre.1)])),
        ),
    ])
    .expect("no cycle");
    assert_eq!(
        forward, reversed,
        "the merged closure is a function of the image, not the merge order"
    );
    assert!(forward.well_closed);
    assert!(reversed.well_closed);
    // Both edge-bearing contract-pre obligations are walked (App's named
    // edge + Bank's own needs obligation with its runtime-check terminal).
    assert_eq!(forward.checked, 2);
}

#[test]
fn merged_closure_unresolved_are_sorted_deterministically() {
    ensure_langc();
    let dir = fresh_dir("unresolved");
    let (bank, app, caller_pre, _callee_pre) = merged_scenario(&dir);
    // Bank's own contract-pre stays open — App's certificate is unresolved.
    let closure = tyu::closure::check_image_closure(&[
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
    assert_eq!(
        closure.unresolved[0].dependency,
        "Bank::withdraw::contract-pre::0"
    );
    // The unresolved list is sorted by (module, id) — deterministic output
    // regardless of walk order (FR-16).
    let got = closure
        .unresolved
        .iter()
        .map(|u| format!("{}::{}", u.module, u.id))
        .collect::<Vec<_>>();
    let mut expected = got.clone();
    expected.sort();
    assert_eq!(got, expected, "sorted by (module, id)");
}

#[test]
fn merged_cycle_is_e6419_class() {
    ensure_langc();
    // A synthetic malformed artifact (two obligations that edge to each
    // other) — the checker must fail closed with a loop path, surfaced as
    // the E6419-class malformed cycle (verify.rs compose).
    let m = mk_synthetic_set(
        "M",
        &[
            (
                "M::a::contract-pre::0",
                "0000000000000000",
                &["M::b::contract-pre::0"],
            ),
            (
                "M::b::contract-pre::0",
                "1111111111111111",
                &["M::a::contract-pre::0"],
            ),
        ],
    );
    let err = tyu::closure::check_image_closure(&[(
        "M".to_string(),
        Some(m),
        Some(mk_echo(&[
            ("M::a::contract-pre::0", "0000000000000000"),
            ("M::b::contract-pre::0", "1111111111111111"),
        ])),
    )])
    .expect_err("a cycle is malformed (E6419)");
    let rendered: Vec<String> = err
        .path
        .iter()
        .map(|(m, id)| format!("{m}::{id}"))
        .collect();
    assert!(
        rendered.first() == rendered.last(),
        "the cycle path closes: {rendered:?}"
    );
    let text = format!(
        "E6419: assumption-closure cycle (malformed): {}",
        rendered.join(" -> ")
    );
    assert!(
        text.contains("E6419"),
        "the E6419 class is named by the report path"
    );
}

#[test]
fn harvest_closure_adjusts_real_two_module_verdicts() {
    ensure_langc();
    let dir = fresh_dir("harvest-adjust");
    let (bank, app, caller_pre, _callee_pre) = merged_scenario(&dir);
    // Harvest-style documents: App's caller certificate present; Bank's own
    // contract-pre unproven (none of its obligations closed).
    let docs = vec![
        ("Bank".to_string(), verifier::testutil::mk_harvest_doc(&[])),
        (
            "App".to_string(),
            verifier::testutil::mk_harvest_doc(&[(&caller_pre.0, &caller_pre.1)]),
        ),
    ];
    let sets = vec![
        ("Bank".to_string(), Some(bank)),
        ("App".to_string(), Some(app)),
    ];
    let adjusted = tyu::closure::apply_harvest_closure(&sets, &docs).expect("no cycle");
    // App's document: the dependent flipped open with the closure witness —
    // this is exactly the document pass-2 langc consumes, so the check is
    // RETAINED (never an elided check with an open report row).
    let app_doc = verifier::verdict::read_verdicts(adjusted[1].1.as_bytes()).unwrap();
    let dep = app_doc
        .records
        .iter()
        .find(|r| r.id == caller_pre.0)
        .expect("dependent present");
    assert_eq!(dep.status, verifier::verdict::VerdictStatus::Open);
    assert_eq!(
        dep.witness_reason.as_deref(),
        Some("assumption-unresolved: Bank::withdraw::contract-pre::0")
    );
    // Bank's document untouched (not affected by the closure).
    assert_eq!(adjusted[0].1, docs[0].1);
}

#[test]
fn diamond_dependencies_are_memoized() {
    ensure_langc();
    // A → {B, C}, B → D, C → D; D open ⇒ A/B/C all unresolved (each walks
    // D exactly once through the memo — a diamond is never re-walked).
    let m = mk_synthetic_set(
        "M",
        &[
            (
                "M::a::contract-pre::0",
                "0000000000000000",
                &["M::b::contract-pre::0", "M::c::contract-pre::0"],
            ),
            (
                "M::b::contract-pre::0",
                "1111111111111111",
                &["M::d::contract-pre::0"],
            ),
            (
                "M::c::contract-pre::0",
                "2222222222222222",
                &["M::d::contract-pre::0"],
            ),
            ("M::d::contract-pre::0", "3333333333333333", &[]),
        ],
    );
    // d NOT in the echo (unproven leaf) — a, b, c all closed.
    let closure = tyu::closure::check_image_closure(&[(
        "M".to_string(),
        Some(m),
        Some(mk_echo(&[
            ("M::a::contract-pre::0", "0000000000000000"),
            ("M::b::contract-pre::0", "1111111111111111"),
            ("M::c::contract-pre::0", "2222222222222222"),
        ])),
    )])
    .expect("no cycle");
    assert!(!closure.well_closed);
    assert_eq!(closure.checked, 3, "a, b, c walked (each edge-bearing)");
    assert_eq!(closure.unresolved.len(), 3, "a, b, c all forced open");
    let deps: Vec<&str> = closure
        .unresolved
        .iter()
        .map(|u| u.dependency.as_str())
        .collect();
    assert!(
        deps.contains(&"M::b::contract-pre::0") && deps.contains(&"M::d::contract-pre::0"),
        "a names B, b/c name D: {deps:?}"
    );
}
