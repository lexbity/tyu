//! NFR-1 timing bench (static-verification.md slice P5): the interval
//! engine's per-word discharge cost. `#[ignore]`d by default — run with
//! `cargo test -p verifier --test timing_bench -- --ignored --nocapture` (the
//! weekly CI job). The gate is a generous log-only bound (the engine is
//! O(ops) per word with ≤ 16 blocks, so real cost is microseconds; the bound
//! guards against accidental blowups, never flakes).

use ir::{BlockId, CmpKind, OpKind, Sig, Span, TypeId};
use verifier::interp::run_cfg;
use std::time::Instant;

fn sr() -> &'static verifier::interp::SubtypeRange<'static> {
    &|tid| if tid.0 == 2 { Some((0, 100)) } else { None }
}

/// A synthetic 16-block, ~90-op word (the caps) with a loop, so the bench
/// exercises the worklist + widening.
fn dense_word() -> ir::Word {
    let mut w = ir::Word {
        name: ir::Atom::new(b"bench").unwrap(),
        sig: Sig {
            in_len: 2,
            out_len: 1,
            ..Sig::empty()
        },
        performs: ir::EffectSet::empty(),
        requires: ir::CapSet::empty(),
        bound: ir::StackBound::ID,
        entry: BlockId(0),
        types: Default::default(),
        type_sizes: Default::default(),
        type_classes: Default::default(),
        apertures: Default::default(),
        subtype_bases: Default::default(),
        blocks: Default::default(),
    };
    let ty = TypeId(0);
    let mut bx = |ops: Vec<OpKind>| {
        let id = BlockId(w.blocks.len() as u16);
        let mut b = ir::Block {
            id,
            entry_stack: Default::default(),
            ops: Default::default(),
        };
        for op in ops {
            b.ops.push(ir::Op { kind: op, span: Span::UNKNOWN }).unwrap();
        }
        w.blocks.push(b).unwrap();
        id
    };
    let _b0 = bx(vec![
        OpKind::LocalSet { slot: 1, ty },
        OpKind::LocalSet { slot: 2, ty },
        OpKind::Br { target: BlockId(1) },
    ]);
    // 6 loop pairs (header+body each) + entry + exit = 14 blocks ≤ 16 cap.
    for i in 1..7 {
        let header = BlockId(i as u16);
        let body = BlockId(i as u16 + 1);
        bx(vec![
            OpKind::LocalGet { slot: 1, ty },
            OpKind::ConstI64(10),
            OpKind::Cmp { out: TypeId(1), kind: CmpKind::Lt },
            OpKind::BrIf { then_tgt: body, else_tgt: header },
        ]);
        bx(vec![
            OpKind::LocalGet { slot: 1, ty },
            OpKind::ConstI64(1),
            OpKind::AddI64,
            OpKind::LocalSet { slot: 1, ty },
            OpKind::LocalGet { slot: 2, ty },
            OpKind::ConstI64(2),
            OpKind::MulI64,
            OpKind::LocalSet { slot: 2, ty },
            OpKind::Br { target: header },
        ]);
    }
    let _exit = bx(vec![OpKind::LocalGet { slot: 2, ty }, OpKind::Ret]);
    w
}

#[test]
#[ignore = "NFR-1 bench — run with -- --ignored (weekly CI job)"]
fn discharge_cost_per_word_is_bounded() {
    let w = dense_word();
    // Warm-up ignored; measure the steady-state per-word discharge.
    let iters = 2000u32;
    let started = Instant::now();
    for _ in 0..iters {
        let _cf = run_cfg(&w, sr());
    }
    let elapsed = started.elapsed();
    let per_word_us = elapsed.as_micros() as f64 / iters as f64;
    eprintln!("interval discharge per word: {per_word_us:.1} µs");
    // Generous log-only bound, far above the observed microseconds (guards
    // accidental algorithmic blowups, never flakes).
    assert!(
        per_word_us < 10_000.0,
        "discharge must stay O(ops)/word (NFR-7); measured {per_word_us:.1} µs"
    );
}