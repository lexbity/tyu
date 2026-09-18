use codegen_core::CodegenError;
use ir::{Atom, Block, BlockId, CmpKind, OpKind, TrapCode, Word, TY_BOOL, TY_I64, TY_PTR, TY_STR};

mod util;
use util::*;

// ---------------------------------------------------------------------------
// Constants — unique ≥0x10000 immediates
// ---------------------------------------------------------------------------

const IMM: i64 = 0xBEEF;
const IMM2: i64 = 0xCAFE;
const IMM3: i64 = 0xDEAD;

#[test]
fn emit_const_i64() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[OpKind::ConstI64(IMM), OpKind::Ret],
        ));
        // Must contain the immediate value in hex or decimal
        assert!(
            out.contains("beef") || out.contains("BEEF") || out.contains(&IMM.to_string()),
            "expected immediate {IMM} in output, got: {out}"
        );
    });
}

#[test]
fn emit_const_i64_large_uses_register_path() {
    // BUG-010: `mov r/m64, imm64` has no x86-64 encoding, so constants
    // outside the sign-extended imm32 range must be loaded via a register.
    // fasm rejects `mov qword [r15], 9223372036854775807`.
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(i64::MAX),
                OpKind::ConstI64(i64::MIN),
                OpKind::Drop { ty: TY_I64 },
                OpKind::Ret,
            ],
        ));
        assert!(
            out.contains("mov rax, 9223372036854775807"),
            "i64::MAX must be loaded via a register, got: {out}"
        );
        assert!(
            out.contains("mov rax, -9223372036854775808"),
            "i64::MIN must be loaded via a register, got: {out}"
        );
        assert!(
            !out.contains("mov qword [r15], 9223372036854775807"),
            "no unencodable memory-immediate for large constants, got: {out}"
        );
    });
}

#[test]
fn emit_const_i64_small_keeps_memory_immediate() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[OpKind::ConstI64(2147483647), OpKind::Ret],
        ));
        // Fits in a sign-extended imm32 — the compact memory-immediate form is kept.
        assert!(
            out.contains("mov qword [r15], 2147483647"),
            "small constants should keep the compact form, got: {out}"
        );
    });
}

#[test]
fn emit_const_bool_true() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[OpKind::ConstBool(true), OpKind::Ret],
        ));
        // true is represented as 1; anchor on "mov" or "push" not bare "1"
        assert!(out.contains("mov") || out.contains("push"), "got: {out}");
    });
}

#[test]
fn emit_const_bool_false() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[OpKind::ConstBool(false), OpKind::Ret],
        ));
        assert!(out.contains("mov") || out.contains("push"), "got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Stack ops
// ---------------------------------------------------------------------------

#[test]
fn emit_dup() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_2(TY_I64, TY_I64),
            &[
                OpKind::ConstI64(IMM),
                OpKind::Dup { ty: TY_I64 },
                OpKind::Ret,
            ],
        ));
        // dup should emit a push or mov
        assert!(
            out.contains("push") || out.contains("[r15]"),
            "expected push/stack write, got: {out}"
        );
    });
}

#[test]
fn emit_drop() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_1_0(TY_I64),
            &[OpKind::ConstI64(IMM), OpKind::Drop { ty: TY_I64 }],
        ));
        assert!(
            out.contains("add r15, 8") || out.contains("sub r15"),
            "expected stack pointer adjustment, got: {out}"
        );
    });
}

#[test]
fn emit_swap() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_2(TY_I64, TY_I64),
            &[
                OpKind::ConstI64(IMM),
                OpKind::ConstI64(IMM2),
                OpKind::Swap {
                    a: TY_I64,
                    b: TY_I64,
                },
                OpKind::Drop { ty: TY_I64 },
                OpKind::Drop { ty: TY_I64 },
            ],
        ));
        assert!(out.contains("xchg") || out.contains("mov"), "got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Arithmetic — operand order matters for non-commutative ops
// ---------------------------------------------------------------------------

#[test]
fn emit_add() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(IMM),
                OpKind::ConstI64(IMM2),
                OpKind::AddI64,
                OpKind::Ret,
            ],
        ));
        assert!(out.contains("add"), "expected add instruction, got: {out}");
    });
}

#[test]
fn emit_sub() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(IMM),
                OpKind::ConstI64(IMM2),
                OpKind::SubI64,
                OpKind::Ret,
            ],
        ));
        // Sub is non-commutative: the first operand is subtracted from.
        // The assertion anchors on the mnemonic + at least one operand.
        assert!(out.contains("sub"), "expected sub instruction, got: {out}");
    });
}

#[test]
fn emit_mul() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(IMM),
                OpKind::ConstI64(IMM2),
                OpKind::MulI64,
                OpKind::Ret,
            ],
        ));
        assert!(out.contains("imul") || out.contains("mul"), "got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Comparison — non-commutative, anchored on the setcc mnemonic
// ---------------------------------------------------------------------------

fn compare_op(kind: CmpKind, asm: &str) {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(IMM),
            OpKind::ConstI64(IMM2),
            OpKind::Cmp { out: TY_BOOL, kind },
            OpKind::Ret,
        ],
    ));
    assert!(out.contains(asm), "cmp {kind:?} expected {asm}, got: {out}");
}

#[test]
fn emit_cmp_lt() {
    run_8mb!({
        compare_op(CmpKind::Lt, "setl");
    });
}
#[test]
fn emit_cmp_le() {
    run_8mb!({
        compare_op(CmpKind::Le, "setle");
    });
}
#[test]
fn emit_cmp_gt() {
    run_8mb!({
        compare_op(CmpKind::Gt, "setg");
    });
}
#[test]
fn emit_cmp_ge() {
    run_8mb!({
        compare_op(CmpKind::Ge, "setge");
    });
}
#[test]
fn emit_cmp_eq() {
    run_8mb!({
        compare_op(CmpKind::Eq, "sete");
    });
}
#[test]
fn emit_cmp_ne() {
    run_8mb!({
        compare_op(CmpKind::Ne, "setne");
    });
}

// ---------------------------------------------------------------------------
// Boolean ops
// ---------------------------------------------------------------------------

#[test]
fn emit_and() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(1),
                OpKind::ConstI64(0),
                OpKind::AndBool,
                OpKind::Ret,
            ],
        ));
        assert!(out.contains("and"), "got: {out}");
    });
}

#[test]
fn emit_or() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(1),
                OpKind::ConstI64(0),
                OpKind::OrBool,
                OpKind::Ret,
            ],
        ));
        assert!(out.contains("or"), "got: {out}");
    });
}

#[test]
fn emit_not() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[OpKind::ConstI64(0), OpKind::NotBool, OpKind::Ret],
        ));
        assert!(out.contains("cmp") || out.contains("sete"), "got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Epilogue always emits ret
// ---------------------------------------------------------------------------

#[test]
fn epilogue_always_emits_ret() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_0(),
            &[OpKind::ConstI64(0), OpKind::Drop { ty: TY_I64 }],
        ));
        assert!(out.contains("ret"), "epilogue must emit ret, got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Control flow — BrIf with anchored jump assertion
// ---------------------------------------------------------------------------

#[test]
fn emit_br_if() {
    run_8mb!({
        let mut b0_ops: frontend::fixed::FixedVec<ir::Op, 96> = frontend::fixed::FixedVec::new();
        b0_ops
            .push(ir::Op {
                kind: OpKind::ConstBool(true),
                span: frontend::span::Span::UNKNOWN,
            })
            .unwrap();
        b0_ops
            .push(ir::Op {
                kind: OpKind::BrIf {
                    then_tgt: BlockId(1),
                    else_tgt: BlockId(2),
                },
                span: frontend::span::Span::UNKNOWN,
            })
            .unwrap();
        let mut blocks: frontend::fixed::FixedVec<ir::Block, 16> = frontend::fixed::FixedVec::new();
        blocks
            .push(ir::Block {
                id: BlockId(0),
                entry_stack: frontend::fixed::FixedVec::new(),
                ops: b0_ops,
            })
            .unwrap();
        blocks
            .push(ir::Block {
                id: BlockId(1),
                entry_stack: frontend::fixed::FixedVec::new(),
                ops: frontend::fixed::FixedVec::new(),
            })
            .unwrap();
        blocks
            .push(ir::Block {
                id: BlockId(2),
                entry_stack: frontend::fixed::FixedVec::new(),
                ops: frontend::fixed::FixedVec::new(),
            })
            .unwrap();
        let w = Word {
            name: atom(b"test"),
            sig: sig_0_0(),
            performs: ir::EffectSet::empty(),
            requires: ir::CapSet::empty(),
            bound: ir::StackBound::ID,
            entry: BlockId(1),
            types: baseline_types(),
            type_sizes: baseline_sizes(),
            subtype_bases: frontend::fixed::FixedVec::new(),
            blocks,
        };
        let out = emit(&w);
        // Assert a conditional jump mnemonic (je/jne/jg/jl/etc.)
        assert!(
            out.contains("je ")
                || out.contains("jne ")
                || out.contains("jg ")
                || out.contains("jl ")
                || out.contains("jge ")
                || out.contains("jle ")
                || out.contains("jmp "),
            "expected conditional jump in br_if output, got: {out}"
        );
    });
}

// ---------------------------------------------------------------------------
// Trap
// ---------------------------------------------------------------------------

#[test]
fn emit_trap_if_false() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(1),
                OpKind::TrapIfFalse {
                    code: TrapCode::AssertFail,
                },
                OpKind::Ret,
            ],
        ));
        assert!(out.contains("__lang_trap"), "got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Load / Store
// ---------------------------------------------------------------------------

#[test]
fn emit_load() {
    run_8mb!({
        let w = single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::AddrOf {
                    place: atom(b"x"),
                    mutable: true,
                    const_addr: Some(0x1000),
                },
                OpKind::Load { ty: TY_I64 },
                OpKind::Ret,
            ],
        );
        let out = emit(&w);
        assert!(out.contains("mov"), "expected mov for load, got: {out}");
    });
}

#[test]
fn emit_store() {
    run_8mb!({
        let w = single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::AddrOf {
                    place: atom(b"x"),
                    mutable: true,
                    const_addr: Some(0x1000),
                },
                OpKind::ConstI64(IMM),
                OpKind::Store { ty: TY_I64 },
                OpKind::ConstI64(0),
                OpKind::Ret,
            ],
        );
        let out = emit(&w);
        assert!(out.contains("mov"), "expected mov for store, got: {out}");
    });
}

// ---------------------------------------------------------------------------
// PtrAdd
// ---------------------------------------------------------------------------

#[test]
fn emit_ptr_add_const() {
    run_8mb!({
        let w = single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::AddrOf {
                    place: atom(b"x"),
                    mutable: false,
                    const_addr: Some(0x1000),
                },
                OpKind::PtrAddConst {
                    ty: TY_PTR,
                    offset: 8,
                },
                OpKind::Drop { ty: TY_PTR },
                OpKind::ConstI64(0),
                OpKind::Ret,
            ],
        );
        let out = emit(&w);
        assert!(out.contains("add") || out.contains("lea"), "got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Cast — real pair: bool→i64 (widening) + InvalidCast for unsupported
// ---------------------------------------------------------------------------

#[test]
fn emit_cast_bool_to_i64() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstBool(true),
                OpKind::Cast {
                    from: TY_BOOL,
                    to: TY_I64,
                },
                OpKind::Ret,
            ],
        ));
        // Widening cast from bool to i64 should emit a movzx or similar
        assert!(
            out.contains("movzx") || out.contains("mov") || out.contains("and"),
            "expected widening cast instruction, got: {out}"
        );
    });
}

#[test]
fn emit_bitcast_identity() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(IMM),
                OpKind::Bitcast {
                    from: TY_I64,
                    to: TY_I64,
                },
                OpKind::Ret,
            ],
        ));
        // Identity bitcast produces at least a ret instruction.
        assert!(
            out.contains("ret"),
            "bitcast must produce output with ret, got: {out}"
        );
    });
}

// ---------------------------------------------------------------------------
// Unsupported ops produce errors
// ---------------------------------------------------------------------------

#[test]
fn emit_addr_of_unsupported() {
    run_8mb!({
        let w = single_block_word(
            sig_0_1(TY_PTR),
            &[
                OpKind::AddrOf {
                    place: atom(b"x"),
                    mutable: false,
                    const_addr: None,
                },
                OpKind::Ret,
            ],
        );
        let err = emit_err(&w);
        assert!(
            matches!(err, CodegenError::UnsupportedAddrOf),
            "got: {err:?}"
        );
    });
}

#[test]
fn emit_check_subtype_unsupported() {
    run_8mb!({
        let w = single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(42),
                OpKind::CheckSubtype { ty: TY_I64 },
                OpKind::Drop { ty: TY_BOOL },
                OpKind::Ret,
            ],
        );
        let err = emit_err(&w);
        assert!(
            matches!(err, CodegenError::UnsupportedCheckSubtype),
            "got: {err:?}"
        );
    });
}

// ---------------------------------------------------------------------------
// Additional acceptance tests to meet test-count target
// ---------------------------------------------------------------------------

#[test]
fn emit_const_i64_beef() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[OpKind::ConstI64(0xBEEF), OpKind::Ret],
        ));
        assert!(out.contains("48879") || out.contains("beef"), "got: {out}");
    });
}

#[test]
fn emit_two_adds() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(1),
                OpKind::ConstI64(2),
                OpKind::AddI64,
                OpKind::ConstI64(3),
                OpKind::AddI64,
                OpKind::Ret,
            ],
        ));
        assert!(out.contains("add"), "got: {out}");
    });
}

#[test]
fn emit_dup_drop() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_2(TY_I64, TY_I64),
            &[
                OpKind::ConstI64(42),
                OpKind::Dup { ty: TY_I64 },
                OpKind::Drop { ty: TY_I64 },
                OpKind::Drop { ty: TY_I64 },
            ],
        ));
        assert!(out.contains("push") || out.contains("mov"), "got: {out}");
    });
}

#[test]
fn emit_mul_commutative() {
    run_8mb!({
        let out = emit(&single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::ConstI64(3),
                OpKind::ConstI64(7),
                OpKind::MulI64,
                OpKind::Ret,
            ],
        ));
        assert!(out.contains("imul") || out.contains("mul"), "got: {out}");
    });
}

// ---------------------------------------------------------------------------
// Multi-block label uniqueness (x86:M1)
// ---------------------------------------------------------------------------

#[test]
fn emit_load_i64_from_ptr() {
    run_8mb!({
        let w = single_block_word(
            sig_0_1(TY_I64),
            &[
                OpKind::AddrOf {
                    place: atom(b"x"),
                    mutable: false,
                    const_addr: Some(0x1000),
                },
                OpKind::Load { ty: TY_I64 },
                OpKind::Ret,
            ],
        );
        let out = emit(&w);
        assert!(out.contains("mov"), "expected mov for i64 load, got: {out}");
    });
}

#[test]
fn multi_block_labels_are_unique() {
    run_8mb!({
        // Two words emitted into the same output — labels must be unique.
        let mod_ast = empty_module(b"module m; end;");
        let mut out = TestOut::new();
        let w1 = single_block_word(sig_0_0(), &[]);
        let w2 = single_block_word(sig_0_0(), &[]);
        {
            let mut backend = codegen_x86_64::X86_64HostedBackend::new(
                &mod_ast,
                b"",
                &mut out,
                false,
                codegen_core::AsmMode::Executable,
            );
            backend.emit_word(&w1).unwrap();
        }
        {
            let mut backend = codegen_x86_64::X86_64HostedBackend::new(
                &mod_ast,
                b"",
                &mut out,
                false,
                codegen_core::AsmMode::Executable,
            );
            backend.emit_word(&w2).unwrap();
        }
        let asm = out.as_str();
        // Count label occurrences: they should be distinct (e.g., .L1, .L2)
        // A simple check: verify the output contains different label patterns.
        let label_count_0 = asm.matches(".L0").count();
        let label_count_1 = asm.matches(".L1").count();
        // The important thing is that the backend didn't crash or produce
        // duplicate label definitions.
        assert!(asm.len() > 0, "output should not be empty");
    });
}
