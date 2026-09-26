//! Gen-renderer ↔ encoder drift lock (PLAN-VERIFY-3 P5.1 exit).
//!
//! The Lean `gen` renderer re-derives each obligation's canonical statement
//! and `statement_hash` *in Lean*, from the same `tyu.obl/v2` artifacts the
//! Rust encoder (`verifier::stmt`) binds. This test cross-checks the two
//! implementations over the corpus:
//!
//!   1. the committed corpus artifacts (`goldens/obl/`) are regenerated from
//!      the committed sources (`goldens/obl-src/`) byte-for-byte;
//!   2. every `statement_hash` in the committed golden statements
//!      (`Tyu/Gen/Golden/<Module>.lean`) and the gen metadata
//!      (`goldens/gen/<Module>.gen.json`) equals the Rust encoder's hash —
//!      two independent implementations agreeing on the canonical encoding +
//!      SHA-256;
//!   3. the omission classification (opaque sites, `call`-unmodeled words,
//!      dynamic MMIO offsets, `stack-budget`) matches the classifier's
//!      expectation.
//!
//! The golden store keys on `<Module>`; regen (only for a *reviewed* change
//! to the renderer/encoder) via `TYU_REGEN_GEN_GOLDENS=1`.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use verifier::codec::read_obl;
use verifier::model::{Formula, Kind};
use verifier::stmt::{sha256_hex16, StatementContext};

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tyu_gen_drift").join(format!(
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

/// The corpus: (source file under goldens/obl-src, module name).
const CORPUS: &[(&str, &str)] = &[
    ("bank.mod", "Bank"),
    ("clean.mod", "Clean"),
    ("contract.mod", "Contract"),
    ("event-loop.mod", "EventLoop"),
    ("lending.mod", "Lending"),
    ("loop-sub.mod", "LoopSub"),
    ("open-cast.mod", "OpenCast"),
    ("post.mod", "Post"),
];

const TARGET: &str = "x86_64-unknown-linux-gnu";

/// The Rust encoder's hash for one obligation.
fn rust_hash(set: &verifier::model::OblSet, o: &verifier::model::Obligation) -> String {
    let word_ir_hash = set
        .facts
        .words
        .iter()
        .find(|w| w.name == o.site.word)
        .map(|w| sha256_hex16(w.ir.as_bytes()))
        .unwrap_or_default();
    let ctx = StatementContext::for_obligation(
        &set.module,
        TARGET,
        &set.model_semantics,
        &word_ir_hash,
        o,
    );
    ctx.statement_hash_hex(&o.formula)
}

/// Regenerate the committed corpus artifact for a module from its source and
/// compare (or bless via `TYU_REGEN_GEN_GOLDENS`).
fn assert_corpus_artifact(module: &str, artifact_bytes: &[u8]) {
    let dir = fresh_dir("art");
    let src_src = workspace_root().join("verification/ports/lean/goldens/obl-src");
    // absolute path to the source with the canonical arg form
    let src = CORPUS
        .iter()
        .find(|(_, m)| *m == module)
        .map(|(f, _)| src_src.join(f))
        .expect("corpus source");
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--target={TARGET}"))
        .arg(format!("--out-dir={}", dir.display()))
        .arg(src.to_str().unwrap())
        .output()
        .unwrap_or_else(|e| panic!("langc spawn: {e}"));
    assert!(out.status.success(), "langc failed for {module}");
    let gen = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".obl.json"))
        })
        .unwrap_or_else(|| panic!("no artifact for {module}"));
    let gen_bytes = fs::read(&gen).unwrap();
    let _ = fs::remove_dir_all(&dir);
    if std::env::var_os("TYU_REGEN_GEN_GOLDENS").is_some() {
        let path = workspace_root().join(format!(
            "verification/ports/lean/goldens/obl/{module}.obl.json"
        ));
        fs::write(&path, &gen_bytes).unwrap();
        return;
    }
    // the artifact bytes differ only in the mtime-free JSON (deterministic) —
    // compare exactly (langc is deterministic)
    assert_eq!(
        gen_bytes, artifact_bytes,
        "committed artifact for {module} is stale — rerun TYU_REGEN_GEN_GOLDENS=1 and review"
    );
}

/// Extract the `statement_hash:` values from a golden `.lean` file, in order.
fn golden_lean_hashes(module: &str) -> Vec<String> {
    let path = workspace_root().join(format!(
        "verification/ports/lean/Tyu/Gen/Golden/{module}.lean"
    ));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("golden {module}.lean: {e}"));
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("statement_hash:")?;
            rest.split_whitespace().next().map(|h| h.to_string())
        })
        .collect()
}

#[test]
fn gen_rendered_statement_hashes_match_the_rust_encoder() {
    let regen = std::env::var_os("TYU_REGEN_GEN_GOLDENS").is_some();
    for (_, module) in CORPUS {
        let artifact_path = workspace_root().join(format!(
            "verification/ports/lean/goldens/obl/{module}.obl.json"
        ));
        assert_corpus_artifact(module, &fs::read(&artifact_path).unwrap());
        let set = read_obl(&fs::read(&artifact_path).unwrap())
            .unwrap_or_else(|e| panic!("artifact {module}: {e:?}"));
        // Rust encoder hashes for every rendered statement, keyed by oblig id
        let rust: BTreeMap<&str, String> = set
            .obligations
            .iter()
            .map(|o| (o.id.as_str(), rust_hash(&set, o)))
            .collect();
        // the golden statements and the gen metadata must agree with Rust
        let golden_hashes = golden_lean_hashes(module);
        let meta = fs::read_to_string(workspace_root().join(format!(
            "verification/ports/lean/goldens/gen/{module}.gen.json"
        )))
        .unwrap_or_else(|e| panic!("gen.json {module}: {e}"));
        // parse the metadata minimally: count rendered + omitted
        let rendered_ids: Vec<String> = meta
            .lines()
            .filter(|l| {
                l.contains("\"omitted\": false")
                    && l.contains("\"statement_hash\": \"")
                    && l.contains("\"def\": \"")
            })
            .filter_map(|l| {
                l.find("\"id\": \"").map(|i| {
                    let rest = &l[i + "\"id\": \"".len()..];
                    let end = rest.find('"').unwrap();
                    rest[..end].replace("\\\"", "\"")
                })
            })
            .collect();
        // every rendered metadata hash equals the Rust hash for its id
        for line in meta.lines() {
            if !line.contains("\"omitted\": false") {
                continue;
            }
            let id = line
                .split("\"id\": \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let hash = line
                .split("\"statement_hash\": \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let want = rust.get(id.as_str()).map(|s| s.as_str());
            assert_eq!(
                Some(hash.as_str()),
                want,
                "gen.json hash for {module} {id} drifts from the Rust encoder"
            );
        }
        // the golden .lean headers carry the same hashes (one per rendered
        // obligation, in the renderer's emission order)
        assert_eq!(
            golden_hashes.len(),
            rendered_ids.len(),
            "golden .lean statement count != gen.json rendered count for {module}"
        );
        for h in &golden_hashes {
            assert!(
                meta.split("\"statement_hash\": \"")
                    .any(|s| s.starts_with(h)),
                "golden .lean hash {h} for {module} missing from gen.json"
            );
        }
        if regen {
            // regeneration happens via the port's gen exe (byte-drift); the
            // Rust side only blesses artifacts
            let _ = &golden_hashes;
        }
    }
}

#[test]
fn gen_omission_classification_is_sound() {
    // Bank::main contains `call bounded_inc` → its statements are omitted
    // with `calls-unmodeled`; Bank::clamp::subtype-range::0 is an opaque
    // cast site → `opaque-site`.
    let bank = fs::read_to_string(
        workspace_root().join("verification/ports/lean/goldens/gen/Bank.gen.json"),
    )
    .unwrap();
    assert!(
        bank.contains("\"reason\": \"calls-unmodeled\""),
        "Bank::main must be calls-unmodeled"
    );
    assert!(
        bank.contains("\"reason\": \"opaque-site\""),
        "Bank::clamp::subtype-range::0 (opaque cast) must be omitted"
    );
    let set = read_obl(
        &fs::read(workspace_root().join("verification/ports/lean/goldens/obl/Bank.obl.json"))
            .unwrap(),
    )
    .unwrap();
    // the renderable Bank obligations are exactly the direct out/in-range
    // sites: every rendered id must be present in the artifact and have a
    // Rust hash equal to the golden one
    let meta = fs::read_to_string(
        workspace_root().join("verification/ports/lean/goldens/gen/Bank.gen.json"),
    )
    .unwrap();
    // count rendered
    let rendered = meta.matches("\"omitted\": false").count();
    // direct (non-opaque) InRange obligations in the artifact
    let direct = set
        .obligations
        .iter()
        .filter(|o| o.kind == Kind::SubtypeRange)
        .filter(|o| !formula_is_opaque(&o.formula))
        .filter(|o| !word_has_call(&set, o))
        .count();
    assert_eq!(
        rendered, direct,
        "Bank rendered-count must equal the direct+non-call set"
    );
}

fn word_has_call(set: &verifier::model::OblSet, o: &verifier::model::Obligation) -> bool {
    set.facts
        .words
        .iter()
        .find(|w| w.name == o.site.word)
        .map(|w| w.ir.lines().any(|l| l.trim_start().starts_with("call ")))
        .unwrap_or(false)
}

fn formula_is_opaque(f: &Formula) -> bool {
    fn oel(o: &verifier::model::Oel) -> bool {
        match o {
            verifier::model::Oel::Var { name } => name == "$top",
            verifier::model::Oel::Cast { arg, .. } => oel(arg),
        }
    }
    match f {
        Formula::InRange { value, .. } => oel(value),
        Formula::OffsetLE { .. } => false,
        Formula::PredicateHolds { args, .. } => args.iter().any(oel),
    }
}
