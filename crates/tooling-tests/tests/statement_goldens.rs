//! Statement goldens (PLAN-VERIFY-3 P1.3, §6.4).
//!
//! The canonical statement encodings + `statement_hash` are pinned per triple
//! over the positive corpus (`ci/verify-corpus/*.mod`), so the band rule's
//! "statement hashes MUST be stable within a toolchain band" has an
//! enforcement point: any encoder perturbation — an altered canonical key
//! order, a changed formula rendering, a drifted `word_ir_hash` — flips the
//! golden gate (negative control run-once: injecting one whitespace into the
//! canonical writer makes this test fail).
//!
//! Determinism is re-proven here too: regen twice → byte-identical.
//!
//! Bless a deliberate change with `TYU_REGEN_STATEMENT_GOLDENS=1` and review
//! the diff — a golden update is a reviewed commit (band rule §Q4 item 3).

mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use verifier::codec::read_obl;
use verifier::model::OblSet;
use verifier::stmt::{sha256_hex16, StatementContext};

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_statement_goldens")
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

/// The positive corpus: every module `--emit=obligations` builds and declares
/// a DISTINCT module (the golden store is keyed by `<Module>.stmt.json`, so a
/// fixture whose module name collides with another cannot be pinned — the
/// tooling corpus's `module Main;` fixtures all share one name and are
/// therefore exercised by `obl_v2_roundtrip` instead). The set below is the
/// verification corpus (the G19/G20/G22 gate's own corpus) plus the rich
/// inline fixtures.
use common::{POSITIVE_FIXTURES, RICH_FIXTURES, TRIPLES};

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

/// The golden document bytes for one module × triple — hand-rolled JSON,
/// deterministic (fixed key order; serde_json's Map sorts keys, so the output
/// is stable across runs and platforms).
fn golden_bytes(set: &OblSet, target: &str) -> Vec<u8> {
    let mut out = String::with_capacity(2048);
    out.push_str("{\n  \"schema\": \"tyu.stmt-goldens/1.0\",\n  \"semantics\": \"");
    let _ = write!(out, "{}", set.semantics);
    out.push_str("\",\n  \"stmt\": \"");
    let _ = write!(out, "{}", set.stmt);
    out.push_str("\",\n  \"target\": \"");
    let _ = write!(out, "{}", target);
    out.push_str("\",\n  \"model_semantics\": \"");
    let _ = write!(out, "{}", set.model_semantics);
    out.push_str("\",\n  \"module\": \"");
    let _ = write!(out, "{}", set.module);
    out.push_str("\",\n  \"statements\": [");
    for (i, o) in set.obligations.iter().enumerate() {
        if i != 0 {
            out.push_str(",\n    ");
        } else {
            out.push_str("\n    ");
        }
        let word_ir_hash = set
            .facts
            .words
            .iter()
            .find(|w| w.name == o.site.word)
            .map(|w| sha256_hex16(w.ir.as_bytes()))
            .unwrap_or_default();
        let ctx = StatementContext::for_obligation(
            &set.module,
            target,
            &set.model_semantics,
            &word_ir_hash,
            o,
        );
        out.push_str("{\"id\": ");
        let _ = write!(out, "{}", json_str(&o.id));
        out.push_str(", \"id_hash\": ");
        let _ = write!(out, "{}", json_str(&o.id_hash));
        out.push_str(", \"statement_hash\": ");
        let _ = write!(out, "{}", json_str(&ctx.statement_hash_hex(&o.formula)));
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

/// Validate the golden document's structural contract (§6.4): the header
/// stanza is complete and every statement record carries a non-empty
/// `id`/`id_hash` and a 64-char lowercase-hex `statement_hash`. Runs against
/// the freshly generated bytes and (in compare mode) the committed file, so a
/// corrupt or truncated golden is diagnosed as itself, not as a drift.
fn validate_golden_document(bytes: &[u8], module: &str, target: &str) {
    let v: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| {
        panic!("statement golden for {module}/{target} is not valid JSON: {e}")
    });
    assert_eq!(v["schema"], "tyu.stmt-goldens/1.0", "golden schema");
    assert_eq!(v["target"], target, "golden target");
    assert_eq!(v["module"], module, "golden module");
    assert!(!v["semantics"].as_str().unwrap_or_default().is_empty());
    assert!(!v["stmt"].as_str().unwrap_or_default().is_empty());
    let stmts = v["statements"]
        .as_array()
        .unwrap_or_else(|| panic!("golden for {module}/{target} must carry a statements array"));
    for s in stmts {
        let id = s["id"]
            .as_str()
            .unwrap_or_else(|| panic!("missing statement id in {module}/{target}"));
        let id_hash = s["id_hash"].as_str().unwrap_or_default();
        let hash = s["statement_hash"].as_str().unwrap_or_default();
        assert!(!id.is_empty(), "statement id must be non-empty");
        assert!(
            !id_hash.is_empty(),
            "statement id_hash must be non-empty: {id}"
        );
        assert_eq!(hash.len(), 64, "statement_hash must be 64 hex chars: {id}");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "statement_hash must be lowercase hex: {id} = {hash}"
        );
    }
}

#[test]
fn statement_goldens_green_across_all_triples_and_corpus() {
    // If a golden set is entirely missing (fresh checkout), fail loudly
    // asking for regen rather than silently passing 0 fixtures.
    let regen = std::env::var_os("TYU_REGEN_STATEMENT_GOLDENS").is_some();
    let expected_per_triple = POSITIVE_FIXTURES.len() + RICH_FIXTURES.len();
    // Golden-store keying is `by module` — a fixture set whose modules
    // collide on a triple would clobber one golden. Assert distinctness.
    let mut distinct: Vec<(String, String)> = Vec::new(); // (module, target)
    for fixture in POSITIVE_FIXTURES {
        for &target in TRIPLES {
            let dir = fresh_dir(&format!(
                "{}-{target}",
                Path::new(fixture).file_stem().unwrap().to_str().unwrap()
            ));
            let set = compile_artifact(&dir, &workspace_root().join(fixture), target);
            record(&mut distinct, &set.module, target);
            let bytes = golden_bytes(&set, target);
            validate_golden_document(&bytes, &set.module, target);
            let path = assert_golden_bytes(regen, &bytes, &set.module, target);
            if !regen {
                // The committed file must itself satisfy the document contract.
                validate_golden_document(&std::fs::read(&path).unwrap(), &set.module, target);
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    // The rich inline fixtures (typed casts, authored-intent contracts).
    for &(name, source) in RICH_FIXTURES {
        for &target in TRIPLES {
            let dir = fresh_dir(&format!("{name}-{target}"));
            let mod_path = dir.join(format!("{name}.mod"));
            std::fs::write(&mod_path, source).unwrap();
            let set = compile_artifact(&dir, &mod_path, target);
            let _ = std::fs::remove_file(&mod_path);
            record(&mut distinct, &set.module, target);
            let bytes = golden_bytes(&set, target);
            validate_golden_document(&bytes, &set.module, target);
            let path = assert_golden_bytes(regen, &bytes, &set.module, target);
            if !regen {
                validate_golden_document(&std::fs::read(&path).unwrap(), &set.module, target);
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    // Every triple is fully covered, and no two fixtures share a module on a
    // triple (a collision would silently clobber a golden — the store keys on
    // `<Module>.stmt.json`).
    for &target in TRIPLES {
        let per = distinct.iter().filter(|(_, t)| t == target).count();
        assert_eq!(
            per, expected_per_triple,
            "triple {target}: statement goldens must cover the whole corpus ({expected_per_triple} distinct modules)"
        );
    }
}

/// Record `(module, target)`, panic on a duplicate module within a triple —
/// the golden store keys on `<Module>.stmt.json`, so a collision is a bug in
/// the fixture set, detected at gate time (not as a silent clobber).
fn record(seen: &mut Vec<(String, String)>, module: &str, target: &str) {
    assert!(
        !seen.iter().any(|(m, t)| m == module && t == target),
        "golden corpus collision: module `{module}` appears twice for `{target}` — the store keys on <Module>.stmt.json"
    );
    seen.push((module.to_string(), target.to_string()));
}

/// Assert + optionally bless the golden bytes for (module, triple).
fn assert_golden_bytes(enabled_regen: bool, bytes: &[u8], module: &str, target: &str) -> PathBuf {
    let path = workspace_root()
        .join("test-goldens/statements")
        .join(target)
        .join(format!("{module}.stmt.json"));
    if enabled_regen {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, bytes).unwrap();
        return path;
    }
    let expected = std::fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "missing statement golden {}: {err}; rerun with TYU_REGEN_STATEMENT_GOLDENS=1",
            path.display()
        )
    });
    assert_eq!(
        expected, bytes,
        "statement golden drifted for {module} on {target} — band rule §Q4; review and bless"
    );
    path
}

#[test]
fn statement_hashes_differ_across_triples() {
    // The same obligation statement has DIFFERENT hashes per (triple,
    // model_semantics) — §Q3: a statement proven on one target says nothing
    // about another. Cross-triple equality would be the drift this pins.
    let mut seen: Vec<String> = Vec::new();
    for &target in TRIPLES {
        let dir = fresh_dir(&format!("triples-{target}"));
        let set = compile_artifact(
            &dir,
            &workspace_root().join("ci/verify-corpus/contract.mod"),
            target,
        );
        let mut module_hashes: Vec<String> = Vec::new();
        for o in &set.obligations {
            let word_ir_hash = set
                .facts
                .words
                .iter()
                .find(|w| w.name == o.site.word)
                .map(|w| sha256_hex16(w.ir.as_bytes()))
                .unwrap_or_default();
            let ctx = StatementContext::for_obligation(
                &set.module,
                target,
                &set.model_semantics,
                &word_ir_hash,
                o,
            );
            module_hashes.push(ctx.statement_hash_hex(&o.formula));
        }
        let key = module_hashes.join("|");
        assert!(
            !seen.contains(&key),
            "identical statement hashes across triples — §Q3 relativation broke on {target}"
        );
        seen.push(key);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn golden_regeneration_is_deterministic() {
    // FR-16 extended: two regen runs over identical inputs produce identical
    // golden bytes (regenerate twice into separate files, compare).
    let mut all: Vec<(String, String, Vec<u8>)> = Vec::new();
    for &target in TRIPLES {
        let dir = fresh_dir(&format!("det-{target}"));
        let set = compile_artifact(
            &dir,
            &workspace_root().join("ci/verify-corpus/open-cast.mod"),
            target,
        );
        all.push((
            set.module.clone(),
            target.to_string(),
            golden_bytes(&set, target),
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
    for &target in TRIPLES {
        let dir = fresh_dir(&format!("det2-{target}"));
        let set = compile_artifact(
            &dir,
            &workspace_root().join("ci/verify-corpus/open-cast.mod"),
            target,
        );
        let bytes2 = golden_bytes(&set, target);
        let prior = all
            .iter()
            .find(|(m, t, _)| *t == target && m == &set.module)
            .map(|(_, _, b)| b)
            .expect("prior run recorded");
        assert_eq!(prior, &bytes2, "golden regen must be byte-deterministic");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
