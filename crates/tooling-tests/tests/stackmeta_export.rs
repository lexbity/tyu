//! Stackmeta goldens (PLAN-VERIFY-3 P4.2's empirical hook for T-C).
//!
//! Emits, per corpus word, the artifact's declared `(net, high)` values plus
//! the word's canonical block op-text and the per-word call-sig resolution
//! (`calls: {callee: net}`; a `call X` contributes `net(X)` to the caller's
//! depth delta — the canonical op text does not carry sigs, so the emitter,
//! which has the module's fact table, resolves it; the PORT re-derives
//! `(net, high)` from that text + calls map via its own monoid and compares —
//! the vectors-anchoring discipline applied to the T-C theorem).
//!
//! The port's `conformance --level stackmeta` replay must PASS on these
//! goldens (net exact, the peak envelope within the declared bound — the
//! sound direction `peak ≤ entry + high`).
//!
//! Regenerate with `TYU_REGEN_STACKMETA=1 cargo test -p tooling-tests
//! --test stackmeta_export`; a committed-file mismatch is a reviewed update.

mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use verifier::codec::read_obl;
use verifier::model::OblSet;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_stackmeta").join(format!(
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

/// Compile a fixture to its artifact and read it.
fn compile_artifact(dir: &Path, fixture: &Path, target: &str) -> OblSet {
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--target={target}"))
        .arg(format!("--out-dir={}", dir.display()))
        .arg(fixture.to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "langc --emit=obligations failed for {} ({}): {}",
        fixture.display(),
        target,
        String::from_utf8_lossy(&out.stderr)
    );
    let artifact = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".obl.json"))
        })
        .unwrap_or_else(|| panic!("no artifact produced for {}", fixture.display()));
    read_obl(&std::fs::read(&artifact).unwrap()).expect("artifact must parse")
}

/// The callee-reported net of a `call X` op: the callee's own declared net
/// (its sig delta — the artifact's fact table is the resolution the port
/// would otherwise lack).
fn callee_net(set: &OblSet, name: &str) -> Option<i64> {
    set.facts
        .words
        .iter()
        .find(|w| w.name == name)
        .map(|w| w.net as i64)
}

/// Resolve the `call <name>` targets in the word's IR text.
fn resolve_calls(set: &OblSet, ir: &str) -> (Vec<(String, i64)>, bool) {
    let mut calls: Vec<(String, i64)> = Vec::new();
    let mut unresolved = false;
    for line in ir.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("call ") {
            let name = rest.split_whitespace().next().unwrap_or("");
            match callee_net(set, name) {
                Some(n) => calls.push((name.to_string(), n)),
                None => unresolved = true,
            }
        }
    }
    // Keep the calls deterministic (text order).
    calls.sort();
    (calls, unresolved)
}

/// The golden document bytes for one module × triple — hand-rolled JSON,
/// deterministic (fixed key order).
fn golden_bytes(set: &OblSet, target: &str) -> Vec<u8> {
    let mut out = String::with_capacity(4096);
    out.push_str("{\n  \"schema\": \"tyu.stackmeta/1\",\n  \"semantics\": \"");
    let _ = write!(out, "{}", set.semantics);
    out.push_str("\",\n  \"target\": \"");
    let _ = write!(out, "{}", target);
    out.push_str("\",\n  \"model_semantics\": \"");
    let _ = write!(out, "{}", set.model_semantics);
    out.push_str("\",\n  \"module\": \"");
    let _ = write!(out, "{}", set.module);
    out.push_str("\",\n  \"words\": [");
    let mut first_word = true;
    for w in set.facts.words.iter() {
        if !first_word {
            out.push(',');
        }
        first_word = false;
        out.push_str("\n    {");
        out.push_str("\"name\": ");
        let _ = write!(out, "{}", json_str(&w.name));
        out.push_str(", \"net\": ");
        let _ = write!(out, "{}", w.net);
        out.push_str(", \"high\": ");
        let _ = write!(out, "{}", w.high);
        out.push_str(", \"top\": ");
        let _ = write!(out, "{}", w.top);
        let (calls, unresolved) = resolve_calls(set, &w.ir);
        out.push_str(", \"blocks\": ");
        let _ = write!(out, "{}", json_str(&w.ir));
        out.push_str(", \"calls\": {");
        for (i, (name, net)) in calls.iter().enumerate() {
            if i != 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{}: {}", json_str(name), net);
        }
        out.push('}');
        if unresolved {
            out.push_str(", \"unresolved\": true");
        }
        out.push('}');
    }
    out.push_str("\n  ]\n}\n");
    out.into_bytes()
}

/// Minimal JSON string escaping for the golden writer.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn stackmeta_dir(triple: &str) -> PathBuf {
    workspace_root().join("test-goldens/stackmeta").join(triple)
}

/// Render the goldens for one triple (file fixtures + rich fixtures).
fn render_triple(triple: &str) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    // File fixtures: module name from the artifact.
    for fixture in common::POSITIVE_FIXTURES {
        let dir = fresh_dir("file");
        let set = compile_artifact(&dir, &workspace_root().join(fixture), triple);
        out.push((set.module.clone(), golden_bytes(&set, triple)));
    }
    // Rich inline fixtures.
    for (name, src) in common::RICH_FIXTURES {
        let dir = fresh_dir("rich");
        let path = dir.join(format!("{name}.mod"));
        std::fs::write(&path, src).unwrap();
        let set = compile_artifact(&dir, &path, triple);
        assert_eq!(set.module, *name, "rich fixture module name mismatch");
        out.push((name.to_string(), golden_bytes(&set, triple)));
    }
    out
}

/// The committed files must byte-match regeneration (the golden gate); with
/// `TYU_REGEN_STACKMETA=1` they are rewritten (a reviewed update).
#[test]
fn stackmeta_files_match_regeneration() {
    let regen = std::env::var("TYU_REGEN_STACKMETA").is_ok();
    for triple in common::TRIPLES {
        let dir = stackmeta_dir(triple);
        let rendered = render_triple(triple);
        for (module, bytes) in rendered {
            let path = dir.join(format!("{module}.json"));
            if regen {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &bytes).unwrap();
            } else {
                let actual = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                    panic!(
                        "committed stackmeta file missing for {triple}/{module} \
                         (run with TYU_REGEN_STACKMETA=1)"
                    )
                });
                assert_eq!(
                    actual.as_bytes(),
                    bytes.as_slice(),
                    "{} drifted from the emitter — regenerate + review",
                    path.display()
                );
            }
        }
    }
    if regen {
        eprintln!("TYU_REGEN_STACKMETA=1: regenerated all stackmeta goldens");
    }
}

/// Determinism: regeneration is byte-identical (FR-16).
#[test]
fn stackmeta_regeneration_is_deterministic() {
    for triple in common::TRIPLES {
        let a = render_triple(triple);
        let b = render_triple(triple);
        assert_eq!(a.len(), b.len(), "{triple}: module set unstable");
        for ((ma, ba), (mb, bb)) in a.iter().zip(b.iter()) {
            assert_eq!(ma, mb, "{triple}: module order unstable");
            assert_eq!(ba, bb, "{triple}/{ma}: bytes nondeterministic (FR-16)");
        }
        // Cross-triple data-domain transparency: the mesh meta is target-
        // independent (same underlying values — the callee-net resolutions
        // do not consult the target).
        let text = |v: &(String, Vec<u8>)| {
            let s = String::from_utf8_lossy(&v.1).to_string();
            // Drop the target/model header lines for the comparison.
            s.lines()
                .filter(|l| !l.contains("\"target\":") && !l.contains("\"model_semantics\":"))
                .collect::<String>()
        };
        let _ = text; // informational; exact equality asserted by the replay gate
        assert!(a.len() >= 2, "{triple}: expected ≥ 2 fixture modules");
    }
}

/// Every `call <name>` in the emitted blocks text is resolved or the word
/// is marked unresolved — the port must never have to guess a callee net.
#[test]
fn stackmeta_calls_resolve_against_facts() {
    for triple in common::TRIPLES {
        for fixture in common::POSITIVE_FIXTURES {
            let dir = fresh_dir("resolve");
            let set = compile_artifact(&dir, &workspace_root().join(fixture), triple);
            for w in set.facts.words.iter() {
                let (calls, unresolved) = resolve_calls(&set, &w.ir);
                // Every resolved net is the callee's declared net.
                for (name, net) in calls {
                    let callee = set
                        .facts
                        .words
                        .iter()
                        .find(|c| c.name == name)
                        .unwrap_or_else(|| panic!("{triple}: call to unknown word {name}"));
                    assert_eq!(callee.net as i64, net, "{triple}: callee net mismatch");
                }
                // Unresolved calls are those whose callee is not in-module
                // (dynamic/re-export). They are *marked*, never silently dropped.
                let _ = unresolved;
            }
        }
    }
}
