//! Renderer scope regressions (PLAN-VERIFY-3 P5.1): the statement renderer
//! must (a) render `mmio-bounds` (`OffsetLE`) obligations with the ENCODER-
//! MATCHING canonical + the `offsetWithin` statement — the register-map
//! E6418 break; and (b) classify contract obligations honestly (omitted with
//! the named reason) so the pipeline never hashes a wrong statement.
//!
//! Real langc artifacts drive the port `gen` renderer; every rendered
//! `statement_hash` must equal the Rust encoder's byte-for-byte (the E6418
//! gate).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_renderer_scope")
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

/// A call-free register-map word: the only statements its obligations need
/// are mmio-bounds (OffsetLE) — no regular calls, so the renderer can
/// express them truthfully.
const REGS_MOD: &str = "\
module Regs;

register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;

const scratch = Scratch @ board.scratch;

: main ( -- i64 )
  &!scratch.A 42 as u32 !u32
  &scratch.A @u32 as i64 ;
end;
";

/// A named-predicate contract module: the callee's `needs` check lowers to
/// a real `call pct-in-range` inside the word — the statement-side step
/// cannot run calls faithfully, so the contract obligations are omitted
/// with the documented reason (never a wrong statement, never an E6418).
const CONTRACT_MOD: &str = "\
module P;
export { withdraw } ;

: pct-in-range ( i64 -- i64 bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: withdraw ( i64 -- bool )
  needs [ pct-in-range ]
  drop true ;

end;
";

fn compile_obl(dir: &Path, module: &str, source: &str, extra: &[&str]) -> verifier::model::OblSet {
    let mod_path = dir.join(format!("{module}.mod"));
    fs::write(&mod_path, source).unwrap();
    let mut cmd = Command::new(langc_exe());
    cmd.arg("--emit=obligations")
        .arg("--target=x86_64-unknown-none")
        .arg(format!(
            "--platform={}",
            common::workspace_root()
                .join("platforms/x86_64-unknown-none")
                .display()
        ))
        .arg(format!(
            "--sysroot={}",
            common::workspace_root()
                .join("sysroot/x86_64-unknown-none")
                .display()
        ))
        .arg(format!("--out-dir={}", dir.display()));
    for a in extra {
        cmd.arg(a);
    }
    cmd.arg(mod_path.to_str().unwrap());
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{module} must compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = fs::read(dir.join(format!("{module}.obl.json"))).unwrap();
    verifier::codec::read_obl(&bytes).expect("artifact must round-trip")
}

fn render(set: &verifier::model::OblSet, dir: &Path) -> serde_json::Value {
    let out = dir.join("gen");
    fs::create_dir_all(&out).unwrap();
    let s =
        Command::new(common::workspace_root().join("verification/ports/lean/.lake/build/bin/gen"))
            .arg("--render")
            .arg("--obl")
            .arg(dir.join(format!("{}.obl.json", set.module)))
            .arg("--out")
            .arg(&out)
            .status()
            .unwrap();
    assert!(s.success(), "gen render failed");
    let meta = fs::read(out.join(format!("{}.gen.json", set.module))).unwrap();
    serde_json::from_slice(&meta).expect("gen metadata parses")
}

fn rust_hash(set: &verifier::model::OblSet, o: &verifier::model::Obligation) -> String {
    let wi = set
        .facts
        .words
        .iter()
        .find(|w| w.name == o.site.word)
        .map(|w| verifier::stmt::sha256_hex16(w.ir.as_bytes()))
        .unwrap_or_default();
    let ctx = verifier::stmt::StatementContext::for_obligation(
        &set.module,
        &set.target,
        &set.model_semantics,
        &wi,
        o,
    );
    ctx.statement_hash_hex(&o.formula)
}

/// Every RENDERED row's metadata hash equals the Rust encoder's (the E6418
/// gate) and the mmio-bounds statements actually render.
#[test]
fn mmio_bounds_render_with_encoder_matching_hashes() {
    common::ensure_bins();
    let dir = fresh_dir("mmio");
    let set = compile_obl(&dir, "Regs", REGS_MOD, &[]);
    let meta = render(&set, &dir);
    let rows = meta["statements"].as_array().unwrap();
    let mut rendered_mmio = 0;
    for (i, o) in set.obligations.iter().enumerate() {
        let row = &rows[i];
        assert_eq!(row["id"], o.id);
        let hash = row["statement_hash"].as_str().unwrap_or("");
        let omitted = row["omitted"].as_bool().unwrap_or(true);
        if o.kind == verifier::model::Kind::MmioBounds {
            assert!(
                !omitted,
                "mmio-bounds must render (constant offset): {:?}",
                row
            );
            rendered_mmio += 1;
            assert_eq!(
                hash,
                rust_hash(&set, o),
                "mmio-bounds statement hash must equal the Rust encoder (E6418 gate)"
            );
        } else if !omitted {
            assert_eq!(hash, rust_hash(&set, o), "rendered row hash drift");
        }
    }
    assert!(rendered_mmio >= 1, "expected renderable mmio-bounds rows");
    // The rendered statement is the aperture-bound claim.
    let lean = fs::read_to_string(dir.join("gen").join(format!("{}.lean", set.module))).unwrap();
    assert!(
        lean.contains("Tyu.Gen.Stmt.offsetWithin"),
        "mmio-bounds statements render offsetWithin: {lean}"
    );
}

/// Contract obligations of a named-predicate word are omitted with the
/// documented reason (the word contains the predicate `call` — unfaithful
/// in the statement-side step) — never a wrong statement, and every
/// RENDERED row still hash-matches (the pipeline never E6418s on
/// contracts).
#[test]
fn contract_obligations_are_honestly_classified() {
    common::ensure_bins();
    let dir = fresh_dir("contract");
    let set = compile_obl(&dir, "P", CONTRACT_MOD, &[]);
    let meta = render(&set, &dir);
    let rows = meta["statements"].as_array().unwrap();
    let mut saw_contract = false;
    for (i, o) in set.obligations.iter().enumerate() {
        let row = &rows[i];
        assert_eq!(row["id"], o.id);
        let omitted = row["omitted"].as_bool().unwrap_or(false);
        if matches!(o.kind, verifier::model::Kind::ContractPre) {
            saw_contract = true;
            assert!(
                omitted,
                "named-predicate contract words are calls-bearing — the \
                 contract row must be omitted, not hashed to a wrong statement"
            );
            assert!(
                row["reason"].as_str().is_some(),
                "the omission reason is recorded"
            );
        }
        if !omitted {
            assert_eq!(
                row["statement_hash"].as_str().unwrap_or(""),
                rust_hash(&set, o),
                "every rendered row still hash-matches (never an E6418)"
            );
        }
    }
    assert!(
        saw_contract,
        "the fixture must carry a contract-pre obligation"
    );
}

/// A call-free contract module: `m` carries `needs` + `ensures` over the
/// call-free predicate `pct-in-range`, and its body has no calls either.
const PURE_MOD: &str = "\
module Pure;
subtype Percent = i64 range 0..100;

: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: m ( Percent -- Percent )
  needs [ pct-in-range ]
  ensures [ pct-in-range ]
  1 + as Percent ;
export { m };
end;
";

/// Contract statements RENDER when the word's recorded IR is call-free —
/// the `PredicateHolds` canonical (`args`/`op`/`predicate{ir,ir_hash,
/// module,name}`) must agree with the Rust encoder byte-for-byte, and the
/// rendered text must be the truthful ∀-scheme (pre) / exit-output claim
/// (post). The raw checks-on extraction cannot produce this shape (the
/// contract check itself lowers to a `call`, and such words honestly omit
/// — the sibling test), so the fixture surgically removes the check
/// call/trap pairs: the artifact a pre-check-IR extractor would emit. The
/// renderer is indifferent to provenance; the E6418 gate is the drift lock.
#[test]
fn contract_statements_render_when_the_word_ir_is_call_free() {
    common::ensure_bins();
    let dir = fresh_dir("contract-render");
    let raw = compile_obl(&dir, "Pure", PURE_MOD, &[]);
    assert!(
        raw.facts
            .words
            .iter()
            .any(|w| w.name == "m" && w.ir.lines().any(|l| l.starts_with("call "))),
        "the raw extraction carries the contract-check call"
    );

    // Surgical mutation: drop the check call/trap pairs (JSON-escaped `\\n`
    // in the artifact text) so the word IR is call-free.
    let path = dir.join("Pure.obl.json");
    let text = fs::read_to_string(&path).unwrap();
    let needle = "call pct-in-range\\ntrap_if_false CONTRACT_FAIL\\n";
    assert!(
        text.contains(needle),
        "fixture shape changed — review this test's surgery"
    );
    fs::write(&path, text.replace(needle, "")).unwrap();
    let set = verifier::codec::read_obl(&fs::read(&path).unwrap())
        .expect("the mutated artifact round-trips");
    assert!(
        !set.facts
            .words
            .iter()
            .any(|w| w.name == "m" && w.ir.contains("call ")),
        "mutation applied"
    );

    let meta = render(&set, &dir);
    let rows = meta["statements"].as_array().unwrap();
    let mut saw_pre = false;
    let mut saw_post = false;
    for o in &set.obligations {
        if !matches!(
            o.kind,
            verifier::model::Kind::ContractPre | verifier::model::Kind::ContractPost
        ) {
            continue;
        }
        let row = rows
            .iter()
            .find(|r| r["id"] == o.id.as_str())
            .unwrap_or_else(|| panic!("row present for {}", o.id));
        assert!(
            !row["omitted"].as_bool().unwrap_or(true),
            "a call-free contract row renders: {}",
            o.id
        );
        assert_eq!(
            row["statement_hash"].as_str().unwrap_or(""),
            rust_hash(&set, o),
            "the PredicateHolds canonical agrees with the Rust encoder ({}): {} vs {}",
            o.id,
            row["statement_hash"],
            rust_hash(&set, o)
        );
        if matches!(o.kind, verifier::model::Kind::ContractPre) {
            saw_pre = true;
        } else {
            saw_post = true;
        }
    }
    assert!(saw_pre && saw_post, "both contract classes exercised");

    // The rendered text: post claims the exit-state outputs; pre is the
    // ∀-scheme over abstract argument values.
    let lean = fs::read_to_string(dir.join("gen").join("Pure.lean")).unwrap();
    assert!(
        lean.contains("predicateHolds Pure_pct_in_range"),
        "lean: {lean}"
    );
    assert!(
        lean.contains("outputAt σf 0"),
        "post args are concrete exit outputs: {lean}"
    );
    assert!(
        lean.contains("a_0 : Int"),
        "pre args are the ∀-scheme's abstract values: {lean}"
    );

    // Elaboration tier (port package built): the rendered contract
    // statements must compile against the port's Tyu environment.
    let port = common::workspace_root().join("verification/ports/lean");
    if port.join(".lake/build/lib/lean/Tyu.olean").is_file() {
        let out = Command::new("lake")
            .current_dir(&port)
            .args(["env", "lean"])
            .arg(dir.join("gen").join("Pure.lean"))
            .output()
            .expect("lake spawn");
        assert!(
            out.status.success(),
            "rendered contract statements must elaborate: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    } else {
        eprintln!("skipping elaboration (port package not built)");
    }
}
