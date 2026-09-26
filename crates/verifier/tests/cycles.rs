//! Cycle-structure (SCC) tests (PLAN-VERIFY-3 §Q9, P1.2).
//!
//! `verifier::model::compute_cycles` is the deterministic
//! mutual-reachability SCC computation a loop-bearing word's statement
//! renderer inducts against. Hand-built block CFGs pin the exact output:
//! nested cycles, disjoint cycles, the single-block self-loop, and the
//! acyclic word.

use ir::{Atom, BlockId, CmpKind, OpKind, Sig, TypeId};
use verifier::model::{compute_cycles, Cycle};

/// Build a word whose blocks are exactly `blocks: Vec<(BlockId, Vec<OpKind>)>`
/// in order; every listed block id is used verbatim (ids may be sparse, as
/// the IR permits non-contiguous ids).
fn word(blocks: &[(u16, Vec<OpKind>)]) -> ir::Word {
    let mut w = ir::Word {
        name: Atom::new(b"probe").unwrap(),
        sig: Sig::empty(),
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
    for &(id, ref ops) in blocks {
        let mut b = ir::Block {
            id: BlockId(id),
            entry_stack: Default::default(),
            ops: Default::default(),
        };
        for (i, op) in ops.iter().enumerate() {
            b.ops
                .push(ir::Op {
                    kind: *op,
                    span: ir::Span::UNKNOWN,
                })
                .unwrap_or_else(|_| panic!("op {i} too many"));
        }
        w.blocks
            .push(b)
            .unwrap_or_else(|_| panic!("too many blocks"));
    }
    w
}

#[test]
fn acyclic_word_has_no_cycles() {
    // b0 -> b1 -> b2 (linear): no cycle.
    let w = word(&[
        (0, vec![OpKind::Br { target: BlockId(1) }]),
        (1, vec![OpKind::Br { target: BlockId(2) }]),
        (2, vec![OpKind::Ret]),
    ]);
    assert_eq!(compute_cycles(&w), Vec::<Cycle>::new());
    assert_eq!(compute_cycles(&w), vec![]);
}

#[test]
fn single_block_self_loop_is_a_cycle() {
    // b0 -> b0: the bounded loop-on-one-block cycle.
    let w = word(&[(0, vec![OpKind::Br { target: BlockId(0) }])]);
    assert_eq!(
        compute_cycles(&w),
        vec![Cycle {
            index: 1,
            blocks: vec!["b0".to_string()],
        }]
    );
}

#[test]
fn two_block_back_edge_is_a_cycle() {
    // b0 -> b1 -> b0 (header/body): the canonical loop SCC.
    let w = word(&[
        (
            0,
            vec![OpKind::BrIf {
                then_tgt: BlockId(1),
                else_tgt: BlockId(2),
            }],
        ),
        (1, vec![OpKind::Br { target: BlockId(0) }]),
        (2, vec![OpKind::Ret]),
    ]);
    assert_eq!(
        compute_cycles(&w),
        vec![Cycle {
            index: 1,
            blocks: vec!["b0".to_string(), "b1".to_string()],
        }]
    );
}

#[test]
fn nested_cycles_merge_into_one_scc() {
    // b0 -> b1 -> b2 -> b1 (inner loop) and b2 -> b0 (outer back-edge):
    // b0/b1/b2 form ONE SCC.
    let w = word(&[
        (0, vec![OpKind::Br { target: BlockId(1) }]),
        (
            1,
            vec![OpKind::BrIf {
                then_tgt: BlockId(2),
                else_tgt: BlockId(3),
            }],
        ),
        (2, vec![OpKind::Br { target: BlockId(1) }]),
        (3, vec![OpKind::Ret]),
    ]);
    // b0 -> b1 -> b2 -> b1 has no back edge to b0 yet — but b1/b2 form one
    // SCC (b2 → b1 and b1 → b2 via the BrIf).
    assert_eq!(
        compute_cycles(&w),
        vec![Cycle {
            index: 1,
            blocks: vec!["b1".to_string(), "b2".to_string()],
        }]
    );
    // With the outer back-edge, b0 joins the SCC.
    let w = word(&[
        (0, vec![OpKind::Br { target: BlockId(1) }]),
        (
            1,
            vec![OpKind::BrIf {
                then_tgt: BlockId(2),
                else_tgt: BlockId(3),
            }],
        ),
        (2, vec![OpKind::Br { target: BlockId(0) }]),
        (3, vec![OpKind::Ret]),
    ]);
    assert_eq!(
        compute_cycles(&w),
        vec![Cycle {
            index: 1,
            blocks: vec!["b0".to_string(), "b1".to_string(), "b2".to_string()],
        }],
        "nested cycles collapse into one SCC"
    );
}

#[test]
fn disjoint_cycles_get_deterministic_ordinals() {
    // Two independent loops: {b0,b1} and {b3,b4} (b2 is a pass-through).
    let w = word(&[
        (0, vec![OpKind::Br { target: BlockId(1) }]),
        (1, vec![OpKind::Br { target: BlockId(0) }]),
        (2, vec![OpKind::Br { target: BlockId(3) }]),
        (3, vec![OpKind::Br { target: BlockId(4) }]),
        (4, vec![OpKind::Br { target: BlockId(3) }]),
    ]);
    assert_eq!(
        compute_cycles(&w),
        vec![
            Cycle {
                index: 1,
                blocks: vec!["b0".to_string(), "b1".to_string()],
            },
            Cycle {
                index: 2,
                blocks: vec!["b3".to_string(), "b4".to_string()],
            },
        ]
    );
}

#[test]
fn sparse_and_duplicated_block_ids_are_handled() {
    // Block ids are not contiguous (the IR allows it); duplicates in the
    // input are collapsed without panic.
    let w = word(&[
        (7, vec![OpKind::Br { target: BlockId(9) }]),
        (9, vec![OpKind::Br { target: BlockId(7) }]),
    ]);
    assert_eq!(
        compute_cycles(&w),
        vec![Cycle {
            index: 1,
            blocks: vec!["b7".to_string(), "b9".to_string()],
        }],
    );
}

#[test]
fn branch_without_cycle_has_no_cycle() {
    let w = word(&[
        (
            0,
            vec![OpKind::BrIf {
                then_tgt: BlockId(1),
                else_tgt: BlockId(2),
            }],
        ),
        (1, vec![OpKind::Ret]),
        (2, vec![OpKind::Ret]),
    ]);
    assert!(
        compute_cycles(&w).is_empty(),
        "if/else is a DAG, not a cycle"
    );
}

// Silence unused-import warnings for the cmo/type helpers used to keep the
// word builders type-correct.
#[allow(dead_code)]
fn _unused_ty() -> TypeId {
    TypeId(0)
}
#[allow(dead_code)]
fn _unused_cmp() -> CmpKind {
    CmpKind::Lt
}
