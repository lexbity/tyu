//! Fragment-vector conformance corpus (PLAN-VERIFY-3 P9.3).
//!
//! The pure-fragment *source* surface (`Tyu/Src.lean` ↔ `verifier::src_interp`)
//! had no mechanical Lean↔Rust pin: each side was internally drift-locked
//! (Lean: the T-S theorems; Rust: `cross_surface.rs` + the transcription
//! table test), but nothing pinned the *mnemonic↔op mapping and the step
//! semantics across the two languages — the fragment analogue of the IR
//! surface's `tyu.vec/1` conformance vectors was missing.
//!
//! This file closes it the same way the IR surface did: a committed,
//! schema-versioned corpus (`crates/verifier/test-vectors/fragment/index.json`,
//! `tyu.fragvec/1`) of *program → trace* observations. The programs are
//! rendered as canonical `--emit=ir` block text ([`blocks_to_text`]) with the
//! expected observable traces derived from the Rust engine (both surfaces
//! must agree at generation time):
//!
//! - the concrete *source-level* run ([`run_word`]) and
//! - the concrete run of the transcribed IR ops ([`ir_run_word`])
//!
//! The committed file is the pin: the port parses the SAME text with its own
//! parser (`Tyu.Gen.Render.parseSrcBlocks`) and must reproduce the committed
//! traces byte-for-byte (`conformance --level fragment`, wired into
//! `ci/port.sh`); `crates/verifier/tests/cross_surface.rs` executes the same
//! file on the Rust side. A drift in the fragment semantics on either side —
//! including a wrong `swap`, the first real bug this corpus caught — diverges
//! the corpus.
//!
//! Regenerate with `TYU_REGEN_FRAG_VECTORS=1 cargo test -p verifier
//! --test fragment_vectors`; a committed-file mismatch is a reviewed update.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use verifier::src_interp::{blocks_to_text, ir_run_word, run_word, Mem, SrcOp, State};

/// The fragment-vector schema identifier.
const FRAG_SCHEMA: &str = "tyu.fragvec/1";
/// The triple the corpus is authored against (i64-concrete; the fragment
/// data domain is target-transparent — the runtime emulates 64-bit
/// arithmetic on every target, §Q3 note).
const FRAG_TRIPLE: &str = "x86_64-unknown-none";

/// One fragment program: its canonical block text + metadata + the expected
/// observable trace (equivalence class of the observable: termination and
/// the exit stack; memory is observed through the programs whose exit stack
/// depends on it — store→load replay, the load oracle, the MMIO oracle).
#[derive(Clone, Debug)]
struct FragVec {
    id: &'static str,
    class: &'static str,
    blocks: Vec<Vec<SrcOp>>,
    entry: usize,
    fuel: usize,
    stack: Vec<i64>,
    locals_cap: usize,
    load_oracle: i64,
    mmio_oracle: i64,
}

impl FragVec {
    fn mem(&self) -> Mem {
        Mem {
            cells: Vec::new(),
            load_oracle: self.load_oracle,
            mmio_oracle: self.mmio_oracle,
        }
    }

    fn state(&self) -> State {
        let mut st = State::fresh(self.locals_cap);
        st.stack.extend_from_slice(&self.stack);
        st
    }

    /// The expected trace, derived from the engine. The two surfaces MUST
    /// agree here: a divergence is a transcription bug, not a golden value.
    fn expect(&self) -> (bool, Vec<i64>) {
        let mem = self.mem();
        let st = self.state();
        let src = run_word(&self.blocks, self.entry, self.fuel, &mem, &st);
        let ir = ir_run_word(&self.blocks, self.entry, self.fuel, &mem, &st);
        match (src, ir) {
            (Some(a), Some(b)) => {
                assert_eq!(
                    a.stack, b.stack,
                    "{}: surfaces disagree on the exit stack of a terminating run",
                    self.id
                );
                (true, a.stack)
            }
            (None, None) => (false, Vec::new()),
            (a, b) => panic!(
                "{}: surfaces disagree on termination (src={:?}, ir={:?})",
                self.id, a, b
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The corpus programs (the same shapes `cross_surface.rs` hand-ran, plus the
// op-coverage words). Expected traces are derived at generation; the
// committed file is the golden all three consumers (Rust generator gate,
// cross_surface.rs, the port) check against.
// ---------------------------------------------------------------------------

fn clean_main() -> Vec<Vec<SrcOp>> {
    // The `Clean.main` corpus word: `const 1; const 2; add; const 3; add;
    // const 4; add; local_set 1; local_get 1; ret` — terminates with `10`.
    vec![vec![
        SrcOp::ConstInt(1),
        SrcOp::ConstInt(2),
        SrcOp::Add,
        SrcOp::ConstInt(3),
        SrcOp::Add,
        SrcOp::ConstInt(4),
        SrcOp::Add,
        SrcOp::LocalSet(1),
        SrcOp::LocalGet(1),
        SrcOp::Ret,
    ]]
}

fn poll() -> Vec<Vec<SrcOp>> {
    // `EventLoop.poll`: `const 1; drop; ret` — terminates with the empty
    // stack.
    vec![vec![SrcOp::ConstInt(1), SrcOp::Drop, SrcOp::Ret]]
}

fn const_42() -> Vec<Vec<SrcOp>> {
    // `const 42; ret` — the Sum worked-example shape.
    vec![vec![SrcOp::ConstInt(42), SrcOp::Ret]]
}

/// The loop-fragment word: count up from the entry stack while `< 3`, then
/// `ret` the counter (each entry value exercises a different iteration
/// count, including the body-never-entered case).
fn count_to_three() -> Vec<Vec<SrcOp>> {
    vec![
        vec![SrcOp::LocalSet(1), SrcOp::Br(1)],
        vec![
            SrcOp::LocalGet(1),
            SrcOp::ConstInt(3),
            SrcOp::CmpLt,
            SrcOp::BrIf(2, 3),
        ],
        vec![
            SrcOp::LocalGet(1),
            SrcOp::ConstInt(1),
            SrcOp::Add,
            SrcOp::LocalSet(1),
            SrcOp::Br(1),
        ],
        vec![SrcOp::LocalGet(1), SrcOp::Ret],
    ]
}

fn store_load() -> Vec<Vec<SrcOp>> {
    // `addr value store; addr load; ret` over the scripted memory
    // (store-then-load replay).
    vec![vec![
        SrcOp::ConstInt(0x1234),
        SrcOp::ConstInt(42),
        SrcOp::Store,
        SrcOp::ConstInt(0x1234),
        SrcOp::Load,
        SrcOp::Ret,
    ]]
}

fn load_oracle() -> Vec<Vec<SrcOp>> {
    // A never-written address reads the load oracle (unmodeled memory).
    vec![vec![SrcOp::ConstInt(0x999), SrcOp::Load, SrcOp::Ret]]
}

fn mmio_read() -> Vec<Vec<SrcOp>> {
    // `vol_load; ret` — reads the oracle (§Q13); every concrete oracle value
    // must flow through identically on both surfaces.
    vec![vec![SrcOp::VolLoad, SrcOp::Ret]]
}

fn mmio_with_store() -> Vec<Vec<SrcOp>> {
    // A stored address is consumed by vol_load: `addr vol_load` pops it and
    // pushes the oracle (the aperture read; the address is not observed).
    vec![vec![SrcOp::ConstInt(0x20), SrcOp::VolLoad, SrcOp::Ret]]
}

fn vol_store() -> Vec<Vec<SrcOp>> {
    // `a b vol_store` pops both and leaves nothing observable.
    vec![vec![
        SrcOp::ConstInt(1),
        SrcOp::ConstInt(2),
        SrcOp::VolStore,
        SrcOp::ConstInt(9),
        SrcOp::Ret,
    ]]
}

fn data_ops() -> Vec<Vec<SrcOp>> {
    // The typed-stack + arithmetic data domain — with `swap` made
    // OBSERVABLE: after `swap; sub` the exit value depends on the swapped
    // order (the correct swap gives `[9]`; an identity swap would give
    // `[-3]`), so a swap drift diverges the corpus trace. This word is why
    // the corpus caught the Lean swap bug.
    vec![vec![
        SrcOp::ConstInt(5),
        SrcOp::Dup,
        SrcOp::Mul, // 5² = 25
        SrcOp::ConstInt(100),
        SrcOp::ConstInt(30),
        SrcOp::Sub,   // 100 − 30 = 70
        SrcOp::CmpLe, // 25 ≤ 70 → 1
        SrcOp::ConstInt(7),
        SrcOp::Swap, // [1,7] → [7,1] (top 1); identity keeps [1,7]
        SrcOp::Sub,  // 7 − 1 = 6 (identity: 1 − 7 = −6)
        SrcOp::ConstInt(3),
        SrcOp::Add, // 6 + 3 = 9
        SrcOp::Ret,
    ]]
}

fn cmp_family() -> Vec<Vec<SrcOp>> {
    // Every comparison op on fixed values (hand-computed expectations).
    vec![vec![
        SrcOp::ConstInt(3),
        SrcOp::ConstInt(4),
        SrcOp::CmpGt, // 3 > 4 → 0
        SrcOp::ConstInt(3),
        SrcOp::ConstInt(3),
        SrcOp::CmpGe, // 3 ≥ 3 → 1
        SrcOp::ConstInt(5),
        SrcOp::ConstInt(6),
        SrcOp::CmpNe, // 5 ≠ 6 → 1
        SrcOp::ConstInt(9),
        SrcOp::ConstInt(9),
        SrcOp::CmpEq, // 9 == 9 → 1
        SrcOp::Ret,
    ]]
}

fn bool_ops() -> Vec<Vec<SrcOp>> {
    vec![vec![
        SrcOp::ConstBool(true),
        SrcOp::ConstBool(false),
        SrcOp::AndB, // true ∧ false → 0
        SrcOp::NotB, // → 1
        SrcOp::ConstBool(false),
        SrcOp::OrB, // 1 ∨ false → 1
        SrcOp::Ret,
    ]]
}

fn local_cells() -> Vec<Vec<SrcOp>> {
    vec![vec![
        SrcOp::ConstInt(10),
        SrcOp::LocalSet(0),
        SrcOp::ConstInt(20),
        SrcOp::LocalSet(1),
        SrcOp::LocalGet(0),
        SrcOp::LocalGet(1),
        SrcOp::Add, // 10 + 20
        SrcOp::Ret,
    ]]
}

fn wrap_add() -> Vec<Vec<SrcOp>> {
    // The wrapping boundary: `i64::MAX + 1` wraps to `i64::MIN` — the
    // wrapI64/wrapping_add agreement class.
    vec![vec![
        SrcOp::ConstInt(i64::MAX),
        SrcOp::ConstInt(1),
        SrcOp::Add,
        SrcOp::Ret,
    ]]
}

fn nonterm_loop() -> Vec<Vec<SrcOp>> {
    // Fuel exhaustion: a self-loop never terminates within any budget.
    vec![vec![SrcOp::Br(0)]]
}

/// The full corpus in authored order (stable — the golden ordering).
fn corpus() -> Vec<FragVec> {
    vec![
        FragVec {
            id: "clean-main",
            class: "corpus-word",
            blocks: clean_main(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 2,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "poll",
            class: "corpus-word",
            blocks: poll(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "const-42",
            class: "corpus-word",
            blocks: const_42(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "count-to-three-from-empty",
            class: "loop",
            blocks: count_to_three(),
            entry: 0,
            fuel: 12,
            stack: Vec::new(),
            locals_cap: 4,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "count-to-three-from-2",
            class: "loop",
            blocks: count_to_three(),
            entry: 0,
            fuel: 12,
            stack: vec![2],
            locals_cap: 4,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "count-to-three-from-5",
            class: "loop",
            blocks: count_to_three(),
            entry: 0,
            fuel: 12,
            stack: vec![5],
            locals_cap: 4,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "store-load",
            class: "memory",
            blocks: store_load(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "load-oracle",
            class: "memory",
            blocks: load_oracle(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: -3,
            mmio_oracle: 0,
        },
        FragVec {
            id: "mmio-read",
            class: "mmio",
            blocks: mmio_read(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 255,
        },
        FragVec {
            id: "mmio-with-store",
            class: "mmio",
            blocks: mmio_with_store(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 7,
        },
        FragVec {
            id: "vol-store",
            class: "mmio",
            blocks: vol_store(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "data-ops",
            class: "data",
            blocks: data_ops(),
            entry: 0,
            fuel: 16,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "cmp-family",
            class: "data",
            blocks: cmp_family(),
            entry: 0,
            fuel: 12,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "bool-ops",
            class: "data",
            blocks: bool_ops(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "local-cells",
            class: "data",
            blocks: local_cells(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 2,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "wrap-add-i64-max",
            class: "boundary",
            blocks: wrap_add(),
            entry: 0,
            fuel: 8,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
        FragVec {
            id: "nonterm-self-loop",
            class: "boundary",
            blocks: nonterm_loop(),
            entry: 0,
            fuel: 3,
            stack: Vec::new(),
            locals_cap: 0,
            load_oracle: 0,
            mmio_oracle: 0,
        },
    ]
}

// ---------------------------------------------------------------------------
// JSON serialization (deterministic; no maps, fixed order, mirrors the
// `tyu.vec/1` corpus style).
// ---------------------------------------------------------------------------

fn frag_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join("fragment")
}

fn render_json() -> String {
    let programs = corpus();
    assert!(!programs.is_empty(), "fragment corpus is empty");
    let mut c = Vec::new();
    c.push(format!(
        "{{\n  \"schema\": \"{FRAG_SCHEMA}\",\n  \"triple\": \"{FRAG_TRIPLE}\",\n  \"programs\": ["
    ));
    for (i, v) in programs.iter().enumerate() {
        let comma = if i + 1 < programs.len() { "," } else { "" };
        let (term, stack) = v.expect();
        let mut expect = format!("{{\"terminates\":{term}");
        if term {
            let parts: Vec<String> = stack.iter().map(|x| x.to_string()).collect();
            let _ = write!(expect, ",\"exit_stack\":[{}]", parts.join(","));
        }
        expect.push('}');
        let stack_text = v
            .stack
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",");
        c.push("    {".to_string());
        c.push(format!("      \"id\": \"{}\",", json_esc(v.id)));
        c.push(format!("      \"class\": \"{}\",", json_esc(v.class)));
        c.push(format!(
            "      \"blocks\": \"{}\",",
            json_esc(&blocks_to_text(&v.blocks))
        ));
        c.push(format!("      \"entry\": {},", v.entry));
        c.push(format!("      \"fuel\": {},", v.fuel));
        c.push(format!("      \"stack\": [{stack_text}],"));
        c.push(format!("      \"locals_cap\": {},", v.locals_cap));
        c.push(format!("      \"load_oracle\": {},", v.load_oracle));
        c.push(format!("      \"mmio_oracle\": {},", v.mmio_oracle));
        c.push(format!("      \"expect\": {expect}"));
        c.push(format!("    }}{comma}"));
    }
    c.push("  ]\n}".to_string());
    c.join("\n")
}

fn json_esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The committed corpus must byte-match regeneration (the golden gate).
#[test]
fn fragment_corpus_files_match_regeneration() {
    let dir = frag_dir();
    let path = dir.join("index.json");
    if std::env::var("TYU_REGEN_FRAG_VECTORS").is_ok() {
        fs::create_dir_all(&dir).expect("create fragment vector dir");
        fs::write(&path, render_json()).expect("write fragment corpus");
        eprintln!("TYU_REGEN_FRAG_VECTORS=1: regenerated the fragment corpus");
        return;
    }
    let expected = render_json();
    let actual = fs::read_to_string(&path)
        .expect("fragment corpus file present (run with TYU_REGEN_FRAG_VECTORS=1 to regenerate)");
    assert_eq!(
        actual,
        expected,
        "fragment corpus drifted from the engine — regenerate + review ({})",
        path.display()
    );
}

/// Determinism: regeneration is byte-identical (FR-16's determinism gate,
/// applied to the fragment surface).
#[test]
fn fragment_corpus_regeneration_is_deterministic() {
    assert_eq!(render_json(), render_json());
}

/// Coverage: every fragment op variant appears in the corpus (a fragment-op
/// addition without a vector would silently exempt the new op from the
/// Lean↔Rust pin until a bug found it).
#[test]
fn fragment_corpus_covers_every_fragment_op() {
    let programs = corpus();
    let mut seen = std::collections::BTreeSet::new();
    for p in &programs {
        for block in &p.blocks {
            for op in block {
                seen.insert(op.mnemonic());
            }
        }
    }
    let variants: Vec<SrcOp> = vec![
        SrcOp::ConstInt(0),
        SrcOp::ConstBool(false),
        SrcOp::Dup,
        SrcOp::Drop,
        SrcOp::Swap,
        SrcOp::Add,
        SrcOp::Sub,
        SrcOp::Mul,
        SrcOp::CmpLt,
        SrcOp::CmpLe,
        SrcOp::CmpGt,
        SrcOp::CmpGe,
        SrcOp::CmpEq,
        SrcOp::CmpNe,
        SrcOp::AndB,
        SrcOp::OrB,
        SrcOp::NotB,
        SrcOp::Load,
        SrcOp::Store,
        SrcOp::VolLoad,
        SrcOp::VolStore,
        SrcOp::LocalGet(0),
        SrcOp::LocalSet(0),
        SrcOp::Br(0),
        SrcOp::BrIf(0, 1),
        SrcOp::Ret,
    ];
    for v in &variants {
        assert!(
            seen.contains(v.mnemonic()),
            "fragment corpus lacks a vector exercising {}",
            v.to_text()
        );
    }
    // Boundary classes: the wrap and the fuel-exhaustion (non-termination)
    // observations must both be present.
    let ids: Vec<&str> = programs.iter().map(|p| p.id).collect();
    assert!(ids.contains(&"wrap-add-i64-max"), "no wrap boundary vector");
    assert!(
        ids.contains(&"nonterm-self-loop"),
        "no fuel-exhaustion vector"
    );
}
