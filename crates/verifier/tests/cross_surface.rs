//! Cross-surface vectors (PLAN-VERIFY-3 P9.2): corpus words executed in BOTH
//! interpreters — the pure-fragment *source* semantics (`verifier::src_interp`,
//! the Rust mirror of `Tyu/Src.lean`) and the concrete IR semantics of the
//! *transcribed* ops — with observable-trace agreement (the Rust analogue of
//! the T-S theorem `Tyu.Src.transcription_run`).
//!
//! The fragment programs are derived from the committed corpus artifacts
//! (`verification/ports/lean/goldens/obl/*.obl.json`) — the words whose IR
//! is entirely fragment ops:
//!
//!   - `Clean.main`: `const 1; const 2; add; const 3; add; const 4; add;
//!     local_set 1; local_get 1; ret` — terminates with `10`;
//!   - `EventLoop.poll`: `const 1; drop; ret` — terminates with `0`.
//!
//! Plus the synthetic fragment programs exercising every op class (memory
//! through the oracles, MMIO, comparisons, locals, CFG). Every program must
//! execute identically under both surfaces for every tested entry state and
//! budget (fail-closed: divergence is a transcription bug).

use verifier::src_interp::{trace_agrees, Mem, SrcOp, State, ir_run_word, run_word};

/// `Clean.main` from the corpus artifact (all ops fragment).
fn clean_main() -> Vec<Vec<SrcOp>> {
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

/// `EventLoop.poll` from the corpus artifact (all ops fragment).
fn poll() -> Vec<Vec<SrcOp>> {
    vec![vec![SrcOp::ConstInt(1), SrcOp::Drop, SrcOp::Ret]]
}

/// `const 42; ret` — the `Sum` worked-example shape.
fn const_42() -> Vec<Vec<SrcOp>> {
    vec![vec![SrcOp::ConstInt(42), SrcOp::Ret]]
}

/// A loop-fragment word: count up from the entry stack while `< 3`, then
/// `ret` the counter.
fn count_to_three() -> Vec<Vec<SrcOp>> {
    vec![
        // block 0: argument on the stack → local 1, br 1
        vec![SrcOp::LocalSet(1), SrcOp::Br(1)],
        // block 1 (header): `local_get 1; const 3; cmp_lt; br_if (2 = body) (3 = exit)`
        vec![
            SrcOp::LocalGet(1),
            SrcOp::ConstInt(3),
            SrcOp::CmpLt,
            SrcOp::BrIf(2, 3),
        ],
        // block 2 (body): `local_get 1; const 1; add; local_set 1; br 1`
        vec![
            SrcOp::LocalGet(1),
            SrcOp::ConstInt(1),
            SrcOp::Add,
            SrcOp::LocalSet(1),
            SrcOp::Br(1),
        ],
        // block 3 (exit): `local_get 1; ret`
        vec![SrcOp::LocalGet(1), SrcOp::Ret],
    ]
}

/// A memory-word: `addr value store; addr load; ret` over the scripted
/// memory (store-then-load replay).
fn store_load(addr: i64, value: i64) -> Vec<Vec<SrcOp>> {
    vec![vec![
        SrcOp::ConstInt(addr),
        SrcOp::ConstInt(value),
        SrcOp::Store,
        SrcOp::ConstInt(addr),
        SrcOp::Load,
        SrcOp::Ret,
    ]]
}

/// An MMIO word: `vol_load; ret` — reads the oracle (a nondeterministic
/// aperture read, §Q13; every concrete oracle value must agree between the
/// two surfaces).
fn mmio_read() -> Vec<Vec<SrcOp>> {
    vec![vec![SrcOp::VolLoad, SrcOp::Ret]]
}

#[test]
fn corpus_words_agree_and_compute_the_expected_values() {
    // Clean.main: `1 2 + 3 + 4 +` = 10.
    let mem = Mem::new(0, 0);
    let st = State::fresh(4);
    assert!(trace_agrees(&clean_main(), 0, 8, &mem, &st));
    assert_eq!(
        run_word(&clean_main(), 0, 8, &mem, &st).unwrap().stack,
        vec![10],
        "Clean.main's fragment reading: 1 2 + 3 + 4 + = 10"
    );
    assert_eq!(
        ir_run_word(&clean_main(), 0, 8, &mem, &st).unwrap().stack,
        vec![10],
        "the transcribed-IR reading agrees on the value"
    );

    // EventLoop.poll: `1 drop` → empty stack.
    assert!(trace_agrees(&poll(), 0, 8, &mem, &st));
    assert_eq!(run_word(&poll(), 0, 8, &mem, &st).unwrap().stack, vec![]);

    // The Sum worked-example shape.
    assert!(trace_agrees(&const_42(), 0, 8, &mem, &st));
    assert_eq!(
        run_word(&const_42(), 0, 8, &mem, &st).unwrap().stack,
        vec![42]
    );
}

#[test]
fn corpus_words_agree_across_entry_states() {
    let mem = Mem::new(0, 0);
    // Pushing on arbitrary entry stacks; the word's own ops are on top.
    for n in 0..8i64 {
        let mut st = State::fresh(4);
        st.stack.push(1000 + n);
        st.stack.push(n);
        assert!(trace_agrees(&clean_main(), 0, 8, &mem, &st));
        assert_eq!(
            run_word(&clean_main(), 0, 8, &mem, &st).unwrap().stack,
            vec![1000 + n, n, 10],
            "Clean.main appends 10 to the entry stack (locals = return-slot)"
        );
        assert!(trace_agrees(&poll(), 0, 8, &mem, &st));
        assert_eq!(
            run_word(&poll(), 0, 8, &mem, &st).unwrap().stack,
            vec![1000 + n, n],
            "poll drops one value"
        );
    }
}

#[test]
fn memory_and_mmio_words_agree_through_the_oracles() {
    // store-then-load point inside the modeled memory (mirror ApertureMem).
    let addr = 0x1234i64;
    for value in [-7i64, 0, 42, 300, i64::MIN / 2] {
        let blocks = store_load(addr, value);
        let mem = Mem::new(0, 0);
        let st = State::fresh(4);
        assert!(trace_agrees(&blocks, 0, 8, &mem, &st));
        assert_eq!(
            run_word(&blocks, 0, 8, &mem, &st).unwrap().stack,
            vec![value],
            "store→load replays the stored value under both surfaces"
        );
    }
    // A never-written address reads the load oracle (unmodeled memory —
    // `ConcreteMem.load` falls through to the oracle when no store matches).
    let never_written = vec![vec![SrcOp::ConstInt(0x999), SrcOp::Load, SrcOp::Ret]];
    let memk = Mem {
        cells: Vec::new(),
        load_oracle: -3,
        mmio_oracle: 0,
    };
    let st = State::fresh(4);
    assert!(trace_agrees(&never_written, 0, 8, &memk, &st));
    assert_eq!(
        run_word(&never_written, 0, 8, &memk, &st).unwrap().stack,
        vec![-3],
        "unmodeled loads answer the load oracle on both surfaces"
    );

    // MMIO reads: the oracle value flows through both surfaces identically.
    for oracle in [0i64, 1, -5, 255] {
        let blocks = mmio_read();
        let mem = Mem {
            cells: Vec::new(),
            load_oracle: 0,
            mmio_oracle: oracle,
        };
        let st = State::fresh(4);
        assert!(trace_agrees(&blocks, 0, 8, &mem, &st));
        assert_eq!(
            run_word(&blocks, 0, 8, &mem, &st).unwrap().stack,
            vec![oracle]
        );
    }
}

#[test]
fn cf_word_agrees_under_every_budget() {
    // The loop: for entry x < 3 the counter increments to 3; for x ≥ 3 it
    // stays x (the body is never entered). Both surfaces must agree for
    // every entry and budget.
    for x in 0..6i64 {
        for fuel in 0..24usize {
            let mut st = State::fresh(4);
            st.stack.push(x);
            let mem = Mem::new(0, 0);
            assert!(
                trace_agrees(&count_to_three(), 0, fuel, &mem, &st),
                "disagreement at entry {x}, fuel {fuel}"
            );
            if fuel >= 10 {
                let expect = x.max(3);
                assert_eq!(
                    run_word(&count_to_three(), 0, fuel, &mem, &st)
                        .unwrap()
                        .stack,
                    vec![expect]
                );
            }
        }
    }
}

#[test]
fn arithmetic_cmp_bool_word_agrees() {
    // A compound fragment word exercising the data ops:
    //   not; 7; cmp_eq; 2; 3; mul; 1; add; cmp_lt; 1; and_bool; ret
    // (the exact value is irrelevant — only the two surfaces' AGREEMENT on
    // the same stack/memory trace matters).
    let blocks = vec![vec![
        SrcOp::ConstBool(true),
        SrcOp::NotB, // true → false → 0
        SrcOp::ConstInt(7),
        SrcOp::CmpEq, // 0 == 7? no → 0
        SrcOp::ConstInt(2),
        SrcOp::ConstInt(3),
        SrcOp::Mul, // 6
        SrcOp::ConstInt(1),
        SrcOp::Add, // 7
        SrcOp::CmpLt, // 0 < 7 → 1
        SrcOp::ConstInt(1),
        SrcOp::AndB, // 1 ∧ 1 → 1
        SrcOp::Ret,
    ]];
    for n in 0..6i64 {
        let mut st = State::fresh(8);
        st.stack.push(n);
        let mem = Mem::new(7, 9);
        assert!(trace_agrees(&blocks, 0, 12, &mem, &st));
        let r = run_word(&blocks, 0, 12, &mem, &st).unwrap();
        assert_eq!(r.stack, vec![n, 1], "entry stack + the bool result");
    }
}