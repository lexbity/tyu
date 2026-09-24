//! End-to-end interval-discharge tests (static-verification.md slice P5,
//! AC-3/AC-4): the in-tree interval engine discharges provable subtype sites
//! with **no verdicts file** under `tyu build --verify-policy=no-open`, and
//! records `provably_failing` for provably-out-of-range values.
//!
//! C4 note (slice P5, corrected by the P5 audit): the store-site hook
//! (memory.rs typed-store) is wired — a store to a subtype-typed place yields
//! a `subtype-range` obligation and, when open, an emitted range check (FR-6).
//! Subtype-typed stores ARE typecheck-reachable — `resource R : Percent;`
//! plus the canonical `&!R <value> !Percent` shape (see
//! `c4_store_site_is_typecheck_reachable_and_open_via_call_path` below; the
//! earlier "no program can reach a store site" note had missed the resource
//! route: E3501/E3515 only close locals and scoped arrays). What blocks an
//! e2e store-site TRAP test is the backend: typed load/store of
//! subtype-typed widths fails codegen with E8009 (width lookup is
//! primitive-only; the borrow itself lowers). The engine logic is covered by
//! `verifier::tests::soundness_differential` (the value-flow transfer is the
//! same). The AC-5 "provable ranges ⇒ no check" clause is exercised here via
//! the cast/return sites.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

mod common;

fn tyu_exe() -> PathBuf {
    common::bin::resolve("tyu")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-intervals-e2e-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build `source` with tyu and the given extra args; returns (out_dir,
/// stderr, success).
fn build(tag: &str, extra: &[&str], source: &str) -> (PathBuf, String, bool) {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("M.mod");
    fs::write(&mod_path, source).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut cmd = Command::new(tyu_exe());
    cmd.arg("build")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap());
    for a in extra {
        cmd.arg(a);
    }
    let out = cmd.output().expect("tyu invocation");
    (out_dir, String::from_utf8_lossy(&out.stderr).into_owned(), out.status.success())
}

fn report(out_dir: &PathBuf) -> serde_json::Value {
    let bytes = fs::read(out_dir.join("verify-report.json")).expect("report written");
    serde_json::from_slice(&bytes).expect("report parses")
}

const PROVABLE: &str = "\
module Bank;
subtype Percent = i64 range 0..100;
: main ( -- i64 )
  100 50 - as Percent as i64
;
end;
";

const PROVABLY_FAILING: &str = "\
module T;
subtype Percent = i64 range 0..100;
: main ( -- i64 )
  150 as Percent as i64
;
end;
";

const RETURN_CHAIN: &str = "\
module R;
subtype Percent = i64 range 0..100;
: bounded_inc ( Percent -- Percent )
  1 + as Percent
;
: main ( -- Percent )
  100 50 - as Percent bounded_inc
;
end;
";

/// AC-3: a provably-in-range cast chain compiles under `--verify-policy=
/// no-open` with NO verdicts file — the interval engine discharged the site.
#[test]
fn provable_cast_chain_discharges_without_verdicts_file() {
    let (out_dir, stderr, ok) = build("ac3", &["--verify-policy=no-open"], PROVABLE);
    assert!(ok, "no-open must pass with a provable chain: {stderr}");
    let r = report(&out_dir);
    let cls = &r["modules"][0]["classes"]["subtype-range"];
    assert_eq!(cls["total"], 1);
    assert_eq!(cls["discharged"], 1, "interval engine discharged the cast");
    assert_eq!(cls["open"], 0);
    assert_eq!(r["emitted_checks"]["subtype_range"], 0, "no check emitted at a discharged site");
    assert_eq!(r["open"].as_array().unwrap().len(), 0);
    // The in-tree discharge method is recorded in the verdicts echo.
    let echo_dir = out_dir.join(".tyu-verify");
    let echo_file = fs::read_dir(&echo_dir).unwrap().next().unwrap().unwrap().path();
    let echo = verifier::verdict::read_echo(&fs::read(&echo_file).unwrap()).unwrap();
    assert_eq!(echo.verdicts.records.len(), 1);
    assert_eq!(
        echo.verdicts.records[0].method.as_deref(),
        Some("interval"),
        "the engine (not a file) discharged the site"
    );
}

/// The `100 50 - as Percent bounded_inc` chain: main's cast discharges (its
/// operand is [50,50]); the callee's C2 return check discharges (the body's
/// `as Percent` narrows the value back into range); the callee's C1 param and
/// C3 cast stay open (a caller's input is unknown), and main's own C2 return
/// stays open (the `Call` to bounded_inc yields ⊤ outputs — Q4).
#[test]
fn return_chain_discharges_c2_and_keeps_c1() {
    let (out_dir, stderr, ok) = build("retchain", &[], RETURN_CHAIN);
    assert!(ok, "open-ok must build: {stderr}");
    let r = report(&out_dir);
    let cls = &r["modules"][0]["classes"]["subtype-range"];
    assert_eq!(cls["total"], 5, "bounded_inc C1+C3+C2, main C3+C2");
    assert_eq!(cls["discharged"], 2);
    assert_eq!(cls["open"], 3);
    let open_ids: Vec<&str> = r["open"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert!(
        open_ids.iter().any(|id| *id == "R::bounded_inc::subtype-range::0"),
        "C1 param still open: {open_ids:?}"
    );
    assert!(
        open_ids.iter().any(|id| *id == "R::bounded_inc::subtype-range::1"),
        "C3 cast on a ⊤ input still open: {open_ids:?}"
    );
    assert!(
        open_ids.iter().any(|id| *id == "R::main::subtype-range::1"),
        "main's C2 return sees a Call (⊤) and stays open: {open_ids:?}"
    );
    // The discharged sites: bounded_inc's C2 (occurrence 2) and main's C3
    // (occurrence 0) are NOT in the open list.
    assert!(
        !open_ids.iter().any(|id| *id == "R::bounded_inc::subtype-range::2"),
        "bounded_inc's return check discharges: {open_ids:?}"
    );
    assert!(
        !open_ids.iter().any(|id| *id == "R::main::subtype-range::0"),
        "main's cast discharges: {open_ids:?}"
    );
}

/// AC-4 (open-ok half): `150 as Percent` stays open (the check is retained —
/// the runtime trap IS the cast semantics), and the report records it
/// `provably_failing` with the interval reason.
#[test]
fn constant_out_of_range_is_provably_failing_and_open() {
    let (out_dir, stderr, ok) = build("ac4a", &[], PROVABLY_FAILING);
    assert!(ok, "open-ok must build: {stderr}");
    let r = report(&out_dir);
    let cls = &r["modules"][0]["classes"]["subtype-range"];
    assert_eq!(cls["total"], 1);
    assert_eq!(cls["open"], 1, "never a discharge for a provably failing site");
    assert_eq!(r["emitted_checks"]["subtype_range"], 1, "check retained");
    // The report's provably-failing list carries the site + the note.
    let pfi = r["provably_failing"].as_array().unwrap();
    assert_eq!(pfi.len(), 1);
    assert_eq!(pfi[0]["id"], "T::main::subtype-range::0");
    assert!(pfi[0]["note"].as_str().unwrap().contains("target [0, 100]"));
    // The open entry carries the interval reason (FR-18 quality bar).
    let reason = r["open"][0]["reason"].as_str().unwrap();
    assert!(reason.contains("[150, 150]"), "open reason: {reason}");
}

/// AC-4 (no-open half): the same program is rejected under
/// `--verify-policy=no-open` with E6410 listing the open obligation.
#[test]
fn no_open_rejects_provably_failing_constant() {
    let (out_dir, stderr, ok) = build("ac4b", &["--verify-policy=no-open"], PROVABLY_FAILING);
    assert!(!ok, "no-open must reject the retained check: {stderr}");
    assert!(stderr.contains("E6410"), "E6410 diagnostics: {stderr}");
    let r = report(&out_dir);
    assert_eq!(r["policy"], "no-open");
    assert_eq!(r["modules"][0]["classes"]["subtype-range"]["open"], 1);
}

/// The report's honest `emitted_checks` field tracks the discharged sites:
/// under open-ok, a fully-provable module emits zero subtype checks.
#[test]
fn emitted_checks_matches_discharges() {
    let (out_dir, _stderr, ok) = build("emit", &[], PROVABLE);
    assert!(ok);
    let r = report(&out_dir);
    assert_eq!(r["emitted_checks"]["subtype_range"], 0);
    assert_eq!(r["emitted_checks"]["data_stack_guards"], true);
}
/// C4 store-site reachability (slice P5 audit correction): a subtype-typed
/// RESOURCE is declarable and storable — `resource R : Percent;` plus the
/// canonical `&!R <value> !Percent` shape typechecks and the C4 hook records
/// the store obligation. (The earlier "no in-tree program can reach a store
/// site" note was wrong: E3501/E3515 only close locals and scoped arrays.)
/// The value path through a word CALL yields ⊤ at the caller (§7.2 transfer
/// table), so the store site is genuinely OPEN under the in-tree engine.
///
/// What still blocks an e2e store-site TRAP test is the backend, not the
/// language: typed load/store of subtype-typed widths fails codegen with
/// E8009 (`UnknownTypeProperties` — width lookup is primitive-only; the
/// borrow itself lowers fine). Exposing subtype width in the backend is the
/// follow-on that makes AC-5's trap leg authorable.
#[test]
fn c4_store_site_is_typecheck_reachable_and_open_via_call_path() {
    use verifier::model::{Formula, Oel, Provenance};

    let src = "\
module StoreReach;
subtype Percent = i64 range 0..100;
resource R : Percent;
: f ( -- Percent ) 50 as Percent ;
: main ( -- i64 )
  R lock [ &!R f !Percent ]
  0
;
end;
";
    let dir = fresh_dir("c4reach");
    let mod_path = dir.join("StoreReach.mod");
    fs::write(&mod_path, src).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let out = Command::new(common::bin::resolve("langc"))
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .expect("langc invocation");
    assert!(
        out.status.success(),
        "subtype-typed resource store must typecheck: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let set = verifier::codec::read_obl(
        &fs::read(out_dir.join("StoreReach.obl.json")).expect("artifact"),
    )
    .expect("artifact valid");
    // main carries exactly one obligation: the C4 store site (f carries the
    // cast + return sites). Its formula is the $top placeholder over the
    // subtype's range — open provenance, discharged by no one from the
    // artifact alone.
    let main_obls: Vec<_> = set
        .obligations
        .iter()
        .filter(|o| o.site.word == "main")
        .collect();
    assert_eq!(main_obls.len(), 1, "one C4 store site in main");
    assert_eq!(main_obls[0].provenance, Provenance::Opaque);
    match &main_obls[0].formula {
        Formula::InRange { value, lo, hi } => {
            assert_eq!(*lo, 0);
            assert_eq!(*hi, 100);
            assert!(matches!(value, Oel::Var { name } if name == "$top"));
        }
        other => panic!("expected InRange, got {other:?}"),
    }
    let _ = fs::remove_dir_all(&dir);
}
