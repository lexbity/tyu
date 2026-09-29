//! Slice P16.2 — the blame surface (PLAN-VERIFY-3 P16.2, §7.3, §Q17): E6410
//! diagnostics render **witness + intent** — a developer sees *what was being
//! proved* (the declared claim) and *why it was open*, not just a location.
//!
//! The three render classes the plan names:
//!
//! 1. **Interval residue** — `150 as Percent` yields a DefFalse endpoint: the
//!    witness carries the derived interval and the target band
//!    (`interval [150, 150] vs target [0, 100]`).
//! 2. **Opaque-provenance honesty** — a cast fed by a `Call` is `⊤`; the
//!    witness says so explicitly (`interval <top> vs target [0, 100]`) — the
//!    toolchain never hides that it could not see through the call.
//! 3. **Authored intent** — a `needs [ … ] intent "…"` clause's contract-pre
//!    renders the developer's words, not a synthesized default.
//!
//! And the report's `open[]` list carries the intent + subject fields (§Q17 —
//! the honest audit sentence), consumed by the certification package's claims
//! section.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn tyu_exe() -> PathBuf {
    common::bin::resolve("tyu")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tyu-blame-surface-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build `source` with tyu under `--verify-policy=no-open` (the E6410
/// surface); returns (out_dir, stderr, success).
fn build_no_open(dir: &Path, source: &str) -> (String, bool) {
    fs::write(dir.join("M.mod"), source).unwrap();
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let out = Command::new(tyu_exe())
        .arg("build")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg("--verify-policy=no-open")
        .arg(dir.join("M.mod").to_str().unwrap())
        .output()
        .expect("tyu invocation");
    (
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn report(dir: &Path) -> serde_json::Value {
    let bytes = fs::read(dir.join("out/verify-report.json")).expect("report written");
    serde_json::from_slice(&bytes).expect("report parses")
}

/// DefFalse endpoint (interval residue): a constant 150 outside `0..100` is
/// provably failing — the retained check is load-bearing and the witness
/// renders the derived singleton interval against the target band.
const DEF_FALSE_ENDPOINT: &str = "\
module DefF;
subtype Percent = i64 range 0..100;
: main ( -- i64 ) 150 as Percent as i64 ;
export { main } ;
end;
";

/// Opaque provenance: the cast target flows from a `Call` — the interval
/// engine cannot see through it, and the witness is honest about `⊤`.
const OPAQUE_PROVENANCE: &str = "\
module M;
subtype Percent = i64 range 0..100;
: drive ( -- i64 ) 42 ;
: main ( -- i64 ) drive as Percent as i64 ;
export { main } ;
end;
";

/// Authored intent: the developer's `intent "…"` clause travels through the
/// caller-discharge `contract-pre` obligation into the rendering.
const AUTHORED_INTENT: &str = "\
module App;
subtype Percent = i64 range 0..100;
: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;
: withdraw ( Percent -- bool )
  needs [ pct-in-range ] intent \"withdraw never exceeds balance\"
  drop true ;
: drive ( -- i64 ) 42 ;
: main ( -- i64 ) drive as Percent withdraw as i64 ;
export { main } ;
end;
";

#[test]
fn def_false_endpoint_renders_interval_residue_witness() {
    let dir = fresh_dir("deffalse");
    let (stderr, ok) = build_no_open(&dir, DEF_FALSE_ENDPOINT);
    assert!(!ok, "no-open must reject the provably failing site");
    let golden = "  subtype-range — DefF::main.main.0 line 3 — intent: \"value must lie in \
subtype range\" (witness: interval [150, 150] vs target [0, 100])";
    assert!(
        stderr.contains(golden),
        "E6410 must render the interval residue + intent; stderr:\n{stderr}"
    );
    assert!(stderr.contains("E6410"), "E6410 diagnostics present");
    // The report carries the audit sentence (§Q17) — intent + subject.
    let r = report(&dir);
    let open = r["open"].as_array().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["intent"], "value must lie in subtype range");
    assert_eq!(open[0]["subject"], "Percent");
    assert!(open[0]["reason"].as_str().unwrap().contains("[150, 150]"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn opaque_provenance_witness_is_honest_about_top() {
    let dir = fresh_dir("opaque");
    let (stderr, ok) = build_no_open(&dir, OPAQUE_PROVENANCE);
    assert!(!ok, "no-open must reject the ⊤-provenance site");
    let golden = "  subtype-range — M::main.main.0 line 4 — intent: \"value must lie in \
subtype range\" (witness: interval <top> vs target [0, 100])";
    assert!(
        stderr.contains(golden),
        "the ⊤ provenance must be visible, never hidden; stderr:\n{stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn authored_intent_renders_the_developers_words() {
    let dir = fresh_dir("authored");
    let (stderr, _ok) = build_no_open(&dir, AUTHORED_INTENT);
    // The caller-discharge contract-pre obligation carries the authored
    // clause, and its rendering has NO witness suffix (a contract-pre open is
    // a dependency gap, not an interval residue).
    let contract = "  contract-pre — App::main.main.0 line 9 — intent: \"withdraw never \
exceeds balance\"";
    assert!(
        stderr.contains(contract),
        "the authored intent must be the claim surfaced for the contract-pre;\
        stderr:\n{stderr}"
    );
    // The synthesized default also renders, with the opaque-provenance
    // witness, on the same site's cast.
    let cast = "  subtype-range — App::main.main.0 line 9 — intent: \"value must lie in \
subtype range\" (witness: interval <top> vs target [0, 100])";
    assert!(
        stderr.contains(cast),
        "synthesized intent + witness; stderr:\n{stderr}"
    );
    // Report: the authored clause flow into the open record verbatim.
    let r = report(&dir);
    let open = r["open"].as_array().unwrap();
    assert!(
        open.iter()
            .any(|o| o["intent"] == "withdraw never exceeds balance"),
        "the authored intent must reach the report's open[]"
    );
    let _ = fs::remove_dir_all(&dir);
}
