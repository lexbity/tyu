//! G52 — the fork ↔ frozen-upstream fragvec differential (PLAN-WASM-1 §Q1).
//!
//! Every program in the committed `tyu.fragvec/1` corpus
//! (`crates/verifier/test-vectors/fragment/index.json`) is executed by BOTH
//! this crate's fragment engine ([`wasm_repl::frag`], the fork) and the
//! frozen upstream ([`verifier::src_interp`]), and both must reproduce the
//! corpus's committed observable traces (termination + exit stack). The
//! corpus is the rendezvous point (DEC-1): a drift in *either* direction —
//! fork away from upstream, or upstream away from the committed file — fails
//! here, so silent divergence has no path through CI.
//!
//! The test can fail in three distinct ways, each with a distinct panic
//! message:
//!
//!   1. **Corpus mismatch** — the fork's run disagrees with the committed
//!      `expect` trace (corpus drift, or fork drift against the pin);
//!   2. **Live-run mismatch** — the fork's run disagrees with the frozen
//!      upstream's live run on the same text (fork drift);
//!   3. **Selftest** — under the `wasm-selftest` feature the fork's `add_i64`
//!      mnemonic is corrupted and this test MUST fail; `ci/wasm.sh --selftest`
//!      asserts the nonzero exit (the G52 red path, AC-9).

use std::fs;
use std::path::PathBuf;

use common::json::Json;
use verifier::src_interp;
use wasm_repl::frag;

/// The fragment-vector schema identifier (pinned as in `fragment_vectors.rs`).
const FRAG_SCHEMA: &str = "tyu.fragvec/1";

/// The committed corpus lives in the *verifier* crate (the fork reads it
/// read-only — the corpus is the rendezvous point, not a fork-owned copy).
fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../verifier/test-vectors/fragment/index.json")
}

/// Does the test binary include the shared `common` module? (frag_parity is
/// the only user of the JSON reader; the include keeps the module tree flat.)
mod common;

#[test]
fn fragvec_corpus_agrees_with_committed_traces_and_frozen_upstream() {
    let raw = fs::read_to_string(corpus_path()).unwrap_or_else(|e| {
        panic!(
            "fragvec corpus missing at {} ({e}); regenerate with \
             `TYU_REGEN_FRAG_VECTORS=1 cargo test -p verifier --features test-util \
             --test fragment_vectors`",
            corpus_path().display()
        )
    });
    let doc = common::json::parse(&raw).expect("the fragvec corpus must parse as JSON");
    assert_eq!(
        doc.get("schema").and_then(Json::as_str),
        Some(FRAG_SCHEMA),
        "fragvec schema version pinned"
    );

    let programs = doc
        .get("programs")
        .and_then(Json::as_arr)
        .expect("the fragvec corpus has a programs array");
    assert!(!programs.is_empty(), "the fragvec corpus is empty");

    for p in programs {
        let id = p.get("id").and_then(Json::as_str).expect("program id");
        let blocks_text = p
            .get("blocks")
            .and_then(Json::as_str)
            .expect("canonical block text");
        let entry = p.get("entry").and_then(Json::as_i64).expect("entry") as usize;
        let fuel = p.get("fuel").and_then(Json::as_i64).expect("fuel") as usize;
        let locals_cap = p
            .get("locals_cap")
            .and_then(Json::as_i64)
            .expect("locals_cap") as usize;
        let load_oracle = p
            .get("load_oracle")
            .and_then(Json::as_i64)
            .expect("load_oracle");
        let mmio_oracle = p
            .get("mmio_oracle")
            .and_then(Json::as_i64)
            .expect("mmio_oracle");
        let init_stack: Vec<i64> = p
            .get("stack")
            .and_then(Json::as_arr)
            .expect("stack")
            .iter()
            .map(|v| v.as_i64().expect("stack value"))
            .collect();
        let expect_term = p
            .get("expect")
            .and_then(|e| e.get("terminates"))
            .and_then(Json::as_bool)
            .expect("expect.terminates");
        let expect_stack: Option<Vec<i64>> = p
            .get("expect")
            .and_then(|e| e.get("exit_stack"))
            .and_then(Json::as_arr)
            .map(|a| {
                a.iter()
                    .map(|v| v.as_i64().expect("exit_stack value"))
                    .collect()
            });

        // -- the fork's live run ------------------------------------------------
        let fork_blocks = frag::blocks_from_text(blocks_text)
            .unwrap_or_else(|| panic!("{id}: the fork cannot re-parse the canonical block text"));

        // Renderer pin (P1: `blocks_to_text` kept byte-for-byte with the frozen
        // upstream): the committed corpus text *is* the upstream renderer's
        // output, so requiring the fork's renderer to reproduce it exactly pins
        // the fork side of the rendezvous — not just the parser + stepper.
        assert_eq!(
            frag::blocks_to_text(&fork_blocks),
            blocks_text,
            "{id}: the fork's blocks_to_text drifted from the canonical corpus text"
        );

        let fork_mem = frag::Mem {
            cells: Vec::new(),
            load_oracle,
            mmio_oracle,
        };
        let mut fork_st = frag::State::fresh(locals_cap);
        fork_st.stack.extend_from_slice(&init_stack);
        let fork_out = frag::run_word(&fork_blocks, entry, fuel, &fork_mem, &fork_st);

        // -- the frozen upstream's live run -------------------------------------
        let up_blocks = src_interp::blocks_from_text(blocks_text).unwrap_or_else(|| {
            panic!("{id}: the frozen upstream cannot re-parse the canonical text")
        });
        let up_mem = src_interp::Mem {
            cells: Vec::new(),
            load_oracle,
            mmio_oracle,
        };
        let mut up_st = src_interp::State::fresh(locals_cap);
        up_st.stack.extend_from_slice(&init_stack);
        let up_out = src_interp::run_word(&up_blocks, entry, fuel, &up_mem, &up_st);

        // 1. corpus agreement — the fork must reproduce the committed trace.
        if expect_term {
            let want = expect_stack.as_ref().unwrap_or_else(|| {
                panic!("{id}: corpus malformed — terminating expect with no exit_stack")
            });
            let fork_terminal = fork_out.as_ref().unwrap_or_else(|| {
                panic!("{id}: corpus committed termination but the fork does not terminate (fuel or trap)")
            });
            assert_eq!(
                fork_terminal.stack, *want,
                "{id}: the fork's exit stack drifted from the committed corpus trace"
            );
        } else {
            if let Some(s) = &fork_out {
                panic!(
                    "{id}: the corpus committed non-termination but the fork terminates \
                     (exit stack={:?})",
                    s.stack
                );
            }
            assert!(
                expect_stack.is_none(),
                "{id}: corpus malformed — non-terminating expect carries an exit_stack"
            );
        }

        // 2. live-run agreement — the fork must match the frozen upstream.
        match (&fork_out, &up_out) {
            (Some(a), Some(b)) => {
                assert_eq!(
                    a.stack, b.stack,
                    "{id}: the fork diverged from the frozen upstream on the exit stack"
                );
                assert_eq!(
                    a.locals, b.locals,
                    "{id}: the fork diverged from the frozen upstream on the local cells"
                );
            }
            (None, None) => {}
            _ => panic!(
                "{id}: the fork and the frozen upstream disagree on termination \
                 (fork={:?}, upstream={:?})",
                fork_out.is_some(),
                up_out.is_some()
            ),
        }
    }
}
