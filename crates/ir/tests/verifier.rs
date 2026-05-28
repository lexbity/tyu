use frontend::{fixed::FixedVec, span::Span};

use ir::{Atom, Block, BlockId, Op, OpKind, Sig, TypeId, Word, TY_BOOL, TY_EMPTY, TY_I64, TY_PTR, TY_PTR_MUT, TY_STR};

fn atom(bytes: &[u8]) -> Atom {
    Atom::new(bytes).unwrap()
}

fn baseline_types() -> FixedVec<Atom, 64> {
    let mut types = FixedVec::new();
    // Must match the built-in TypeId constants in `ir`.
    types.push(atom(b"")).unwrap(); // 0
    types.push(atom(b"i64")).unwrap(); // 1
    types.push(atom(b"bool")).unwrap(); // 2
    types.push(atom(b"str")).unwrap(); // 3
    types.push(atom(b"ptr")).unwrap(); // 4
    types.push(atom(b"ptr_mut")).unwrap(); // 5
    types.push(atom(b"mmio")).unwrap(); // 6
    types
}

fn baseline_type_sizes() -> FixedVec<u32, 64> {
    let mut sizes = FixedVec::new();
    sizes.push(0).unwrap(); // ""
    sizes.push(8).unwrap(); // i64
    sizes.push(1).unwrap(); // bool
    sizes.push(8).unwrap(); // str (pointer)
    sizes.push(8).unwrap(); // ptr
    sizes.push(8).unwrap(); // ptr_mut
    sizes.push(8).unwrap(); // mmio
    sizes
}

fn sig0_1(out0: TypeId) -> Sig {
    let mut sig = Sig::empty();
    sig.in_len = 0;
    sig.out_len = 1;
    sig.outputs[0] = out0;
    sig
}

fn word_with_single_block(sig: Sig, block_ops: &[OpKind]) -> Word {
    let mut ops: FixedVec<Op, 96> = FixedVec::new();
    for &kind in block_ops {
        ops.push(Op {
            kind,
            span: Span::new(0, 0),
        })
        .unwrap();
    }

    let b0 = Block {
        id: BlockId(0),
        entry_stack: FixedVec::new(),
        ops,
    };

    let mut blocks: FixedVec<Block, 16> = FixedVec::new();
    blocks.push(b0).unwrap();

    Word {
        name: atom(b"w"),
        sig,
        entry: BlockId(0),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    }
}

#[test]
fn verifier_accepts_simple_const_ret() {
    let w = word_with_single_block(sig0_1(TY_I64), &[OpKind::ConstI64(7), OpKind::Ret]);
    ir::verify_word(&w).unwrap();
}

#[test]
fn verifier_rejects_ret_with_wrong_stack_height() {
    let w = word_with_single_block(sig0_1(TY_I64), &[OpKind::Ret]);
    let err = ir::verify_word(&w).unwrap_err();
    assert_eq!(err.code, 9033);
}

#[test]
fn verifier_rejects_drop_type_mismatch() {
    let w = word_with_single_block(sig0_1(TY_I64), &[OpKind::ConstI64(1), OpKind::Drop { ty: TY_BOOL }, OpKind::Ret]);
    let err = ir::verify_word(&w).unwrap_err();
    assert_eq!(err.code, 9012);
}

#[test]
fn verifier_rejects_branch_stack_mismatch() {
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();

    // b0: push i64, br b1 (but b1 expects empty stack)
    let mut b0_ops: FixedVec<Op, 96> = FixedVec::new();
    b0_ops.push(Op {
        kind: OpKind::ConstI64(1),
        span: Span::new(0, 0),
    })
    .unwrap();
    b0_ops.push(Op {
        kind: OpKind::Br { target: BlockId(1) },
        span: Span::new(0, 0),
    })
    .unwrap();
    blocks
        .push(Block {
            id: BlockId(0),
            entry_stack: FixedVec::new(),
            ops: b0_ops,
        })
        .unwrap();

    // b1: empty -> ret (expects empty because sig is ( -- ))
    let mut b1_ops: FixedVec<Op, 96> = FixedVec::new();
    b1_ops.push(Op {
        kind: OpKind::Ret,
        span: Span::new(0, 0),
    })
    .unwrap();
    blocks
        .push(Block {
            id: BlockId(1),
            entry_stack: FixedVec::new(),
            ops: b1_ops,
        })
        .unwrap();

    let mut sig = Sig::empty();
    sig.in_len = 0;
    sig.out_len = 0;

    let w = Word {
        name: atom(b"w"),
        sig,
        entry: BlockId(0),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };

    let err = ir::verify_word(&w).unwrap_err();
    assert_eq!(err.code, 9027);
}

#[test]
fn verifier_rejects_unterminated_block() {
    // Block ends without Ret, Br, or BrIf — must fail with E9035.
    let w = word_with_single_block(sig0_1(TY_I64), &[OpKind::ConstI64(42)]);
    let err = ir::verify_word(&w).unwrap_err();
    assert_eq!(err.code, 9035);
}

#[test]
fn verifier_accepts_terminated_block() {
    // Sanity check: a block that pushes the right value and Rets must pass.
    let w = word_with_single_block(sig0_1(TY_I64), &[OpKind::ConstI64(1), OpKind::Ret]);
    ir::verify_word(&w).unwrap();
}

#[test]
fn verifier_type_pool_baseline_indices_match_constants() {
    let types = baseline_types();
    assert_eq!(types.get(TY_EMPTY.0 as usize).unwrap().as_bytes(), b"");
    assert_eq!(types.get(TY_I64.0 as usize).unwrap().as_bytes(), b"i64");
    assert_eq!(types.get(TY_BOOL.0 as usize).unwrap().as_bytes(), b"bool");
    assert_eq!(types.get(TY_STR.0 as usize).unwrap().as_bytes(), b"str");
    assert_eq!(types.get(TY_PTR.0 as usize).unwrap().as_bytes(), b"ptr");
    assert_eq!(types.get(TY_PTR_MUT.0 as usize).unwrap().as_bytes(), b"ptr_mut");
}
