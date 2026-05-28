use frontend::{fixed::FixedVec, span::Span};

use ir::{Atom, Block, BlockId, Op, OpKind, Sig, TypeId, Word, TY_BOOL, TY_EMPTY, TY_I64, TY_MMIO, TY_PTR, TY_PTR_MUT, TY_STR};

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
            span: Span::UNKNOWN,
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
        span: Span::UNKNOWN,
    })
    .unwrap();
    b0_ops.push(Op {
        kind: OpKind::Br { target: BlockId(1) },
        span: Span::UNKNOWN,
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
        span: Span::UNKNOWN,
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

// ---------------------------------------------------------------------------
// Word-level verification errors
// ---------------------------------------------------------------------------

#[test]
fn verify_word_rejects_missing_entry_block() {
    // Entry BlockId(0) but no block with that id
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();
    blocks
        .push(Block {
            id: BlockId(42),
            entry_stack: FixedVec::new(),
            ops: FixedVec::new(),
        })
        .unwrap();
    let w = Word {
        name: atom(b"w"),
        sig: Sig::empty(),
        entry: BlockId(0),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9001);
}

#[test]
fn verify_word_rejects_entry_stack_height_mismatch() {
    // sig has in_len=1 but entry_stack is empty
    let mut sig = Sig::empty();
    sig.in_len = 1;
    sig.inputs[0] = TY_I64;

    let w = word_with_single_block(sig, &[OpKind::ConstI64(1), OpKind::Ret]);
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9002);
}

#[test]
fn verify_word_rejects_entry_stack_type_mismatch() {
    // entry_stack has wrong type for sig input
    let mut sig = Sig::empty();
    sig.in_len = 1;
    sig.inputs[0] = TY_BOOL;

    let mut entry_stack: FixedVec<TypeId, 32> = FixedVec::new();
    entry_stack.push(TY_I64).unwrap();
    let mut ops: FixedVec<Op, 96> = FixedVec::new();
    ops.push(Op { kind: OpKind::Ret, span: Span::UNKNOWN }).unwrap();

    let mut blocks: FixedVec<Block, 16> = FixedVec::new();
    blocks
        .push(Block {
            id: BlockId(0),
            entry_stack,
            ops,
        })
        .unwrap();
    let w = Word {
        name: atom(b"w"),
        sig,
        entry: BlockId(0),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9003);
}

// ---------------------------------------------------------------------------
// Block-level: ops after termination
// ---------------------------------------------------------------------------

#[test]
fn verify_block_rejects_ops_after_ret() {
    let w = word_with_single_block(Sig::empty(), &[OpKind::Ret, OpKind::ConstI64(0)]);
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9010);
}

#[test]
fn verify_block_rejects_ops_after_br() {
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();
    let mut b0_ops: FixedVec<Op, 96> = FixedVec::new();
    b0_ops.push(Op { kind: OpKind::Br { target: BlockId(1) }, span: Span::UNKNOWN }).unwrap();
    b0_ops.push(Op { kind: OpKind::ConstI64(0), span: Span::UNKNOWN }).unwrap();
    blocks
        .push(Block {
            id: BlockId(0),
            entry_stack: FixedVec::new(),
            ops: b0_ops,
        })
        .unwrap();
    blocks
        .push(Block {
            id: BlockId(1),
            entry_stack: FixedVec::new(),
            ops: FixedVec::new(),
        })
        .unwrap();
    let mut sig = Sig::empty();
    let w = Word {
        name: atom(b"w"),
        sig,
        entry: BlockId(1),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9010);
}

// ---------------------------------------------------------------------------
// Dup type mismatch
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_dup_type_mismatch() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[OpKind::ConstI64(1), OpKind::Dup { ty: TY_BOOL }, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }, OpKind::Ret],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9011);
}

// ---------------------------------------------------------------------------
// Swap type mismatch
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_swap_type_mismatch() {
    // push i64, push i64, swap with wrong types
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::ConstI64(1),
            OpKind::ConstI64(2),
            OpKind::Swap { a: TY_I64, b: TY_BOOL },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Ret,
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9013);
}

// ---------------------------------------------------------------------------
// LocalSet / Cast / Bitcast type mismatch (all use 9016)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_localset_type_mismatch() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[OpKind::ConstI64(1), OpKind::LocalSet { slot: 0, ty: TY_BOOL }, OpKind::ConstI64(1), OpKind::Ret],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9016);
}

#[test]
fn verify_rejects_cast_type_mismatch() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[OpKind::ConstBool(true), OpKind::Cast { from: TY_I64, to: TY_BOOL }, OpKind::Ret],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9016);
}

#[test]
fn verify_rejects_bitcast_type_mismatch() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[OpKind::ConstBool(true), OpKind::Bitcast { from: TY_I64, to: TY_BOOL }, OpKind::Ret],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9016);
}

// ---------------------------------------------------------------------------
// Call stack underflow
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_call_stack_underflow() {
    let mut call_sig = Sig::empty();
    call_sig.in_len = 1;
    call_sig.inputs[0] = TY_I64;
    call_sig.out_len = 0;

    let w = word_with_single_block(
        Sig::empty(),
        &[OpKind::Call { name: atom(b"f"), sig: call_sig, may_suspend: false }],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9017);
}

// ---------------------------------------------------------------------------
// Call input type mismatch (9018)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_call_input_type_mismatch() {
    let mut call_sig = Sig::empty();
    call_sig.in_len = 1;
    call_sig.inputs[0] = TY_BOOL;
    call_sig.out_len = 0;

    let w = word_with_single_block(
        Sig::empty(),
        &[
            OpKind::ConstI64(1),
            OpKind::Call { name: atom(b"f"), sig: call_sig, may_suspend: false },
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9018);
}

// ---------------------------------------------------------------------------
// PtrAddIndex idx not i64 (9018)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_ptr_add_index_wrong_idx_type() {
    // PtrAddIndex expects TY_I64 on top, then a pointer
    let w = word_with_single_block(
        sig0_1(TY_PTR),
        &[
            OpKind::ConstI64(0),
            OpKind::ConstBool(true), // not i64
            OpKind::PtrAddIndex { ty: TY_PTR, scale: 8 },
            OpKind::Drop { ty: TY_PTR },
            OpKind::ConstI64(0),
            OpKind::Ret,
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9018);
}

// ---------------------------------------------------------------------------
// Load with wrong address type (9019)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_load_from_non_pointer() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::ConstI64(42), // not a pointer
            OpKind::Load { ty: TY_I64 },
            OpKind::Ret,
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9019);
}

#[test]
fn verify_rejects_mmio_vol_load_from_non_pointer() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::ConstBool(true), // not a pointer/mmio
            OpKind::MmioVolLoad { ty: TY_I64, place: atom(b"r") },
            OpKind::Ret,
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9019);
}

// ---------------------------------------------------------------------------
// Store value type mismatch (9020)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_store_value_type_mismatch() {
    let w = word_with_single_block(
        Sig::empty(),
        &[
            OpKind::ConstI64(0),
            OpKind::ConstBool(true), // value = bool
            OpKind::Store { ty: TY_I64 }, // expects i64
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9020);
}

// ---------------------------------------------------------------------------
// Store address not mutable (9021)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_store_immutable_ptr() {
    // Store pops value (top) then addr. Push addr first, then value.
    let w = word_with_single_block(
        Sig::empty(),
        &[
            OpKind::AddrOf { place: atom(b"x"), mutable: false, const_addr: None }, // ptr, not ptr_mut
            OpKind::ConstI64(42), // value
            OpKind::Store { ty: TY_I64 },
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9021);
}

// ---------------------------------------------------------------------------
// MmioVolLoadField place != MMIO (9022)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_mmio_load_field_not_mmio() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::ConstI64(42), // not an MMIO place
            OpKind::MmioVolLoadField { reg_ty: TY_I64, field_ty: TY_BOOL, place: atom(b"r"), mask: 0xff, shift: 0 },
            OpKind::Ret,
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9022);
}

// ---------------------------------------------------------------------------
// MmioVolStoreField type mismatch (9023)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_mmio_store_field_type_mismatch() {
    // push mmio place, push wrong value type
    let w = word_with_single_block(
        Sig::empty(),
        &[
            OpKind::MmioPlace { place: atom(b"r"), addr: 0x1000 },
            OpKind::ConstI64(42),
            OpKind::MmioVolStoreField { reg_ty: TY_I64, field_ty: TY_BOOL, place: atom(b"r"), mask: 0xff, shift: 0 },
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9023);
}

// ---------------------------------------------------------------------------
// CheckSubtype type mismatch (9024)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_check_subtype_wrong_type() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::ConstI64(42),
            OpKind::CheckSubtype { ty: TY_BOOL },
            OpKind::Drop { ty: TY_BOOL },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Ret,
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9024);
}

// ---------------------------------------------------------------------------
// TrapIfFalse not bool (9025)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_trap_if_false_not_bool() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::ConstI64(42),
            OpKind::TrapIfFalse { code: ir::TrapCode::AssertFail },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Ret,
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9025);
}

// ---------------------------------------------------------------------------
// Br target not found (9026)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_br_target_not_found() {
    let w = word_with_single_block(Sig::empty(), &[OpKind::Br { target: BlockId(99) }]);
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9026);
}

// ---------------------------------------------------------------------------
// Br target entry stack type mismatch (9028)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_br_target_stack_type_mismatch() {
    // b0 pushes i64, branches to b1 which expects bool
    // sig is ( bool -- ), entry is b1 with entry_stack = [bool]
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();

    let mut b0_ops: FixedVec<Op, 96> = FixedVec::new();
    b0_ops.push(Op { kind: OpKind::ConstI64(1), span: Span::UNKNOWN }).unwrap();
    b0_ops.push(Op { kind: OpKind::Br { target: BlockId(1) }, span: Span::UNKNOWN }).unwrap();
    blocks
        .push(Block {
            id: BlockId(0),
            entry_stack: FixedVec::new(),
            ops: b0_ops,
        })
        .unwrap();

    let mut b1_entry: FixedVec<TypeId, 32> = FixedVec::new();
    b1_entry.push(TY_BOOL).unwrap();
    blocks
        .push(Block {
            id: BlockId(1),
            entry_stack: b1_entry,
            ops: FixedVec::new(),
        })
        .unwrap();

    let mut sig = Sig::empty();
    sig.in_len = 1;
    sig.inputs[0] = TY_BOOL;
    sig.out_len = 0;

    let w = Word {
        name: atom(b"w"),
        sig,
        entry: BlockId(1),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9028);
}

// ---------------------------------------------------------------------------
// BrIf cond not bool (9029)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_brif_cond_not_bool() {
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();

    let mut b0_ops: FixedVec<Op, 96> = FixedVec::new();
    b0_ops.push(Op { kind: OpKind::ConstI64(42), span: Span::UNKNOWN }).unwrap();
    b0_ops.push(Op { kind: OpKind::BrIf { then_tgt: BlockId(1), else_tgt: BlockId(1) }, span: Span::UNKNOWN }).unwrap();
    blocks
        .push(Block {
            id: BlockId(0),
            entry_stack: FixedVec::new(),
            ops: b0_ops,
        })
        .unwrap();
    blocks
        .push(Block {
            id: BlockId(1),
            entry_stack: FixedVec::new(),
            ops: FixedVec::new(),
        })
        .unwrap();

    let w = Word {
        name: atom(b"w"),
        sig: Sig::empty(),
        entry: BlockId(1),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9029);
}

// ---------------------------------------------------------------------------
// BrIf target not found (9030)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_brif_target_not_found() {
    let w = word_with_single_block(
        Sig::empty(),
        &[
            OpKind::ConstBool(true),
            OpKind::BrIf { then_tgt: BlockId(99), else_tgt: BlockId(99) },
        ],
    );
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9030);
}

// ---------------------------------------------------------------------------
// BrIf target stack depth mismatch (9031) — needs multi-block setup
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_brif_target_stack_depth_mismatch() {
    // b0: push bool (cond), push i64 (extra), BrIf to b1
    // BrIf pops the cond (bool), leaving i64 on stack.
    // b1 expects empty entry_stack → depth mismatch (1 vs 0)
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();

    let mut b0_ops: FixedVec<Op, 96> = FixedVec::new();
    b0_ops.push(Op { kind: OpKind::ConstI64(1), span: Span::UNKNOWN }).unwrap(); // extra value BELOW cond
    b0_ops.push(Op { kind: OpKind::ConstBool(true), span: Span::UNKNOWN }).unwrap(); // cond on TOP
    b0_ops.push(Op { kind: OpKind::BrIf { then_tgt: BlockId(1), else_tgt: BlockId(1) }, span: Span::UNKNOWN }).unwrap();
    blocks
        .push(Block {
            id: BlockId(0),
            entry_stack: FixedVec::new(),
            ops: b0_ops,
        })
        .unwrap();
    blocks
        .push(Block {
            id: BlockId(1),
            entry_stack: FixedVec::new(),
            ops: FixedVec::new(),
        })
        .unwrap();

    let w = Word {
        name: atom(b"w"),
        sig: Sig::empty(),
        entry: BlockId(1),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9031);
}

// ---------------------------------------------------------------------------
// BrIf target stack type mismatch (9032)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_brif_target_stack_type_mismatch() {
    // b0: push i64, push bool (cond), BrIf
    // After BrIf pops cond, b1 expects bool but i64 is on stack
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();

    let mut b0_ops: FixedVec<Op, 96> = FixedVec::new();
    b0_ops.push(Op { kind: OpKind::ConstI64(1), span: Span::UNKNOWN }).unwrap(); // value
    b0_ops.push(Op { kind: OpKind::ConstBool(true), span: Span::UNKNOWN }).unwrap(); // cond on top
    b0_ops.push(Op { kind: OpKind::BrIf { then_tgt: BlockId(1), else_tgt: BlockId(1) }, span: Span::UNKNOWN }).unwrap();
    blocks
        .push(Block {
            id: BlockId(0),
            entry_stack: FixedVec::new(),
            ops: b0_ops,
        })
        .unwrap();

    let mut b1_entry: FixedVec<TypeId, 32> = FixedVec::new();
    b1_entry.push(TY_BOOL).unwrap(); // b1 expects bool, but i64 was pushed
    blocks
        .push(Block {
            id: BlockId(1),
            entry_stack: b1_entry,
            ops: FixedVec::new(),
        })
        .unwrap();

    let mut sig = Sig::empty();
    sig.in_len = 1;
    sig.inputs[0] = TY_BOOL;

    let w = Word {
        name: atom(b"w"),
        sig,
        entry: BlockId(1),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9032);
}

// ---------------------------------------------------------------------------
// Ret output type mismatch (9034)
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_ret_output_type_mismatch() {
    // sig says ( -- i64 ) but stack has bool at top
    let w = word_with_single_block(sig0_1(TY_I64), &[OpKind::ConstBool(true), OpKind::Ret]);
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9034);
}

// ---------------------------------------------------------------------------
// Stack underflow / overflow in verifier
// ---------------------------------------------------------------------------

#[test]
fn verify_rejects_stack_underflow() {
    // Pop from empty stack
    let w = word_with_single_block(Sig::empty(), &[OpKind::Drop { ty: TY_I64 }]);
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9098);
}

#[test]
fn verify_handles_stack_overflow() {
    // Exceed the verifier's internal 64-entry stack with 70 pushes.
    let mut ops: FixedVec<Op, 96> = FixedVec::new();
    for _ in 0..70 {
        ops.push(Op { kind: OpKind::ConstI64(0), span: Span::UNKNOWN }).unwrap();
    }
    ops.push(Op { kind: OpKind::Ret, span: Span::UNKNOWN }).unwrap();

    let b0 = Block {
        id: BlockId(0),
        entry_stack: FixedVec::new(),
        ops,
    };
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();
    blocks.push(b0).unwrap();
    let w = Word {
        name: atom(b"w"),
        sig: Sig::empty(),
        entry: BlockId(0),
        types: baseline_types(),
        type_sizes: baseline_type_sizes(),
        blocks,
    };
    // The verifier's stack is [TypeId; 64]; push 65 returns 9099.
    assert_eq!(ir::verify_word(&w).unwrap_err().code, 9099);
}

// ---------------------------------------------------------------------------
// Accept valid ops
// ---------------------------------------------------------------------------

#[test]
fn verify_accepts_addr_of() {
    let w = word_with_single_block(
        sig0_1(TY_PTR),
        &[OpKind::AddrOf { place: atom(b"x"), mutable: false, const_addr: None }, OpKind::Ret],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_mmio_place() {
    let w = word_with_single_block(
        sig0_1(TY_MMIO),
        &[OpKind::MmioPlace { place: atom(b"r"), addr: 0x1000 }, OpKind::Ret],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_scoped_enter() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[OpKind::ScopedEnter { ty: TY_I64, len: 16 }, OpKind::Drop { ty: TY_I64 }, OpKind::ConstI64(0), OpKind::Ret],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_task_spawn() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[OpKind::TaskSpawn { name: atom(b"t"), task_ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }, OpKind::ConstI64(0), OpKind::Ret],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_ptr_add_const() {
    let w = word_with_single_block(
        sig0_1(TY_PTR),
        &[
            OpKind::AddrOf { place: atom(b"x"), mutable: false, const_addr: None },
            OpKind::PtrAddConst { ty: TY_PTR, offset: 8 },
            OpKind::Ret,
        ],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_ptr_add_index() {
    let w = word_with_single_block(
        sig0_1(TY_PTR),
        &[
            OpKind::AddrOf { place: atom(b"x"), mutable: false, const_addr: None },
            OpKind::ConstI64(3),
            OpKind::PtrAddIndex { ty: TY_PTR, scale: 8 },
            OpKind::Ret,
        ],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_load_store() {
    // AddrOf_mut pushes ptr_mut, ConstI64 pushes value, Store pops both.
    // Sig: ( -- i64 ) so after store we need a value on stack for Ret.
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::AddrOf { place: atom(b"x"), mutable: true, const_addr: None },
            OpKind::ConstI64(42),
            OpKind::Store { ty: TY_I64 },
            OpKind::ConstI64(0),
            OpKind::Ret,
        ],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_mmio_load() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::MmioPlace { place: atom(b"r"), addr: 0x1000 },
            OpKind::MmioVolLoad { ty: TY_I64, place: atom(b"r") },
            OpKind::Ret,
        ],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_mmio_store() {
    // MmioStore pops two: value then mmio. Need trailing value for Ret.
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::MmioPlace { place: atom(b"r"), addr: 0x1000 },
            OpKind::ConstI64(0),
            OpKind::MmioVolStore { ty: TY_I64, place: atom(b"r") },
            OpKind::ConstI64(0),
            OpKind::Ret,
        ],
    );
    ir::verify_word(&w).unwrap();
}

#[test]
fn verify_accepts_check_subtype() {
    let w = word_with_single_block(
        sig0_1(TY_I64),
        &[
            OpKind::ConstI64(42),
            OpKind::CheckSubtype { ty: TY_I64 },
            OpKind::Drop { ty: TY_BOOL },
            OpKind::Ret,
        ],
    );
    ir::verify_word(&w).unwrap();
}
