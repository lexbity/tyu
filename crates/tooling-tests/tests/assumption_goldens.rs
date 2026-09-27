//! Assumption-edge goldens (PLAN-VERIFY-3 P8.1): the extractor's
//! cross-module `assumptions` edges are pinned byte-for-byte, per corpus
//! fixture, so the T-CL closure walker (P8.2) executes against a stable,
//! auditable edge surface — and any extractor change that perturbs an edge
//! (a missing edge is an unsound elision, a spurious edge a false `open`;
//! §Q7 rule 1) is a reviewed golden update, never a silent drift.
//!
//! Scenarios, matching `assumption_edges.rs`:
//!   - `edge-present`   (Bank artifact + Bank.def on the path): the caller
//!     `App`'s `contract-pre` carries the NAMED edge to
//!     `Bank::withdraw::contract-pre::0`;
//!   - `edge-degraded`  (only Bank.def — the callee artifact unavailable):
//!     the edge degrades to the conservative `runtime-check` terminal (the
//!     emitted check IS the discharge).
//!
//! `TYU_REGEN_ASSUMPTION_GOLDENS=1` rewrites the store; the double-regen
//! discipline of §P0 (regen → regen → `git diff --exit-code
//! test-goldens/assumptions/`) proves determinism — every run here
//! recompiles fresh fixtures, so a compare run IS a determinism check too.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_assumption_goldens")
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

const TRIPLE: &str = "x86_64-unknown-linux-gnu";

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

fn compile_obl(dir: &Path, module: &str, source: &str) -> verifier::model::OblSet {
    let mod_path = dir.join(format!("{module}.mod"));
    fs::write(&mod_path, source).unwrap();
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--target={TRIPLE}"))
        .arg(format!("--out-dir={}", dir.display()))
        .arg(mod_path.to_str().unwrap())
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

/// The golden document for one scenario: every obligation (sorted by id)
/// with its `assumptions` edges (§6.1 serialization).
fn golden_doc(module: &str, scenario: &str, set: &verifier::model::OblSet) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(b"{\"schema\":\"tyu.assumptions/1\",\"module\":");
    push_str(&mut out, module);
    out.extend_from_slice(b",\"target\":");
    push_str(&mut out, &set.target);
    out.extend_from_slice(b",\"scenario\":");
    push_str(&mut out, scenario);
    out.extend_from_slice(b",\"obligations\":[");
    let mut sorted: Vec<&verifier::model::Obligation> = set.obligations.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    for (i, o) in sorted.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"id\":");
        push_str(&mut out, &o.id);
        out.extend_from_slice(b",\"kind\":");
        push_str(&mut out, o.kind.as_str());
        out.extend_from_slice(b",\"assumptions\":[");
        for (j, e) in o.assumptions.iter().enumerate() {
            if j != 0 {
                out.push(b',');
            }
            match e {
                verifier::model::AssumptionEdge::RuntimeCheck => {
                    out.extend_from_slice(b"\"runtime-check\"");
                }
                verifier::model::AssumptionEdge::Obligation { id, module } => {
                    out.extend_from_slice(b"{\"obligation\":");
                    push_str(&mut out, id);
                    out.extend_from_slice(b",\"module\":");
                    push_str(&mut out, module);
                    out.push(b'}');
                }
            }
        }
        out.extend_from_slice(b"]}");
    }
    out.extend_from_slice(b"]}");
    out
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for b in s.bytes() {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x00..=0x1F => {}
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

fn golden_path(module: &str) -> PathBuf {
    common::workspace_root()
        .join("test-goldens/assumptions")
        .join(format!("{module}.json"))
}

/// Compare-or-regenerate one scenario golden.
fn check_golden(tag: &str, module: &str, scenario: &str, set: &verifier::model::OblSet) {
    assert_eq!(
        set.target, TRIPLE,
        "artifacts must be triple-tagged for the golden store"
    );
    let want = golden_doc(module, scenario, set);
    let path = golden_path(module);
    if std::env::var("TYU_REGEN_ASSUMPTION_GOLDENS").is_ok_and(|v| v == "1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &want).unwrap();
        return;
    }
    let have = fs::read(&path).unwrap_or_else(|e| {
        panic!("golden missing for {tag} — run with TYU_REGEN_ASSUMPTION_GOLDENS=1: {e}")
    });
    assert_eq!(
        have, want,
        "golden drift for {tag} ({module}, {scenario}) — regenerate and REVIEW the diff (band rule, §Q4)"
    );
}

#[test]
fn assumption_edges_byte_exact() {
    common::ensure_bins();
    // Edge-present: Bank artifact + Bank.def on the path → the caller's
    // contract-pre carries the NAMED edge.
    let dir = fresh_dir("present");
    let bank = compile_obl(&dir, "Bank", BANK_MOD);
    fs::write(dir.join("Bank.def"), BANK_DEF).unwrap();
    let app_present = compile_obl(&dir, "App", APP_MOD);
    // Edge-degraded: only Bank.def (no Bank.obl.json) → runtime-check edge.
    let dir_degraded = fresh_dir("degraded");
    fs::write(dir_degraded.join("Bank.def"), BANK_DEF).unwrap();
    let app_degraded = compile_obl(&dir_degraded, "App", APP_MOD);

    check_golden("callee baseline (edge-free)", "Bank", "edge-present", &bank);
    check_golden("caller named edge", "App", "edge-present", &app_present);
    check_golden(
        "caller degraded edge",
        "App-degraded",
        "edge-degraded",
        &app_degraded,
    );
}
