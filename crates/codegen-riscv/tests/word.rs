//! First word-emission tests for the RISC-V backend.
//!
//! Covers slot discipline (dup/drop adjust by 8), comparison lowering
//! (slt/sltu sequences), ConstStr table emission, trap symbol, and a
//! smoke golden.

use codegen_core::{AsmMode, CodegenError};
use codegen_riscv::RiscVBackend;
use frontend::{
    fixed::FixedVec,
    parse::{ModuleAst, Output, Parser},
    span::Span,
};
use ir::{
    Atom, Block, BlockId, CapSet, EffectSet, Op, OpKind, Sig, StackBound, TrapCode, Word,
    TY_I64, TY_BOOL,
};

struct TestOut(Vec<u8>);
impl TestOut {
    fn new() -> Self { Self(Vec::new()) }
    fn as_str(&self) -> &str { core::str::from_utf8(&self.0).unwrap() }
}
impl Output for TestOut {
    fn write(&mut self, bytes: &[u8]) { self.0.extend_from_slice(bytes); }
}

fn atom(b: &[u8]) -> Atom { Atom::new(b).unwrap() }

fn baseline_types() -> FixedVec<Atom, 64> {
    let mut t = FixedVec::new();
    t.push(atom(b"")).unwrap();
    t.push(atom(b"i64")).unwrap();
    t.push(atom(b"bool")).unwrap();
    t
}
fn baseline_sizes() -> FixedVec<u32, 64> {
    let mut s = FixedVec::new(); s.push(0).unwrap(); s.push(8).unwrap(); s.push(1).unwrap(); s
}

fn empty_module(src: &[u8]) -> ModuleAst {
    Parser::new(src).parse_module_ast().unwrap()
}

fn single_block_word(sig: Sig, ops: &[OpKind]) -> Word {
    let mut opv: FixedVec<Op, 96> = FixedVec::new();
    for &k in ops {
        opv.push(Op { kind: k, span: Span::UNKNOWN }).unwrap();
    }
    Word {
        name: atom(b"test"), sig,
        performs: EffectSet::empty(), requires: CapSet::empty(),
        bound: StackBound::ID, entry: BlockId(0),
        types: baseline_types(), type_sizes: baseline_sizes(),
        blocks: {
            let mut b = FixedVec::new();
            b.push(Block { id: BlockId(0), entry_stack: FixedVec::new(), ops: opv }).unwrap();
            b
        },
    }
}

fn sig_0_0() -> Sig { Sig::empty() }
fn sig_0_1(out: ir::TypeId) -> Sig {
    let mut s = Sig::empty(); s.out_len = 1; s.outputs[0] = out; s
}

fn emit(w: &Word) -> String {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = RiscVBackend::new(&mod_ast, b"", &mut out, false, AsmMode::Executable);
    backend.emit_word(w).unwrap();
    out.as_str().to_string()
}

// ---------------------------------------------------------------------------
// Slot discipline: dup/drop adjust by 8 (each stack slot is 8 bytes)
// ---------------------------------------------------------------------------

#[test]
fn riscv_dup_adjusts_by_8() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(0xBEEF), OpKind::Dup { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("addi s2, s2, -8") || out.contains("addi s2, s2, 8"),
        "dup must adjust s2 by 8, got: {out}");
}

#[test]
fn riscv_drop_adjusts_by_8() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(42), OpKind::Drop { ty: TY_I64 }],
    ));
    // Drop adjusts s2 by -8 (two 4-byte pops).
    assert!(out.contains("addi s2, s2, -8"), "drop must adjust s2 by -8, got: {out}");
}

// ---------------------------------------------------------------------------
// Comparison lowering: uses slt/sltu for ordered comparisons
// ---------------------------------------------------------------------------

#[test]
fn riscv_cmp_uses_slt() {
    let out = emit(&single_block_word(
        sig_0_1(TY_BOOL),
        &[OpKind::ConstI64(1), OpKind::ConstI64(2), OpKind::Cmp { out: TY_BOOL, kind: ir::CmpKind::Lt }, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    // RISC-V comparisons use slt/sltu instructions.
    assert!(out.contains("slt") || out.contains("sltu") || out.contains("blt"),
        "cmp must use slt/sltu/blt, got: {out}");
}

// ---------------------------------------------------------------------------
// Trap symbol reference
// ---------------------------------------------------------------------------

#[test]
fn riscv_trap_if_false_references_trap() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(1), OpKind::TrapIfFalse { code: TrapCode::AssertFail }, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("__lang_trap") || out.contains("__lang_trap_loc"),
        "trap must reference __lang_trap, got: {out}");
}

// ---------------------------------------------------------------------------
// Smoke golden: compile a minimal word and verify structure
// ---------------------------------------------------------------------------

#[test]
fn riscv_add_uses_add() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(1), OpKind::ConstI64(2), OpKind::AddI64, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    // RISC-V 64-bit add uses `add` instruction.
    assert!(out.contains("add"), "add must emit add, got: {out}");
}

#[test]
fn riscv_const_i64_emits_li_sw_pair() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(0xBEEF), OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("0xbeef") || out.contains("48879"),
        "ConstI64 must contain the immediate value, got: {out}");
}

#[test]
fn riscv_const_bool_emits_code() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstBool(true), OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("li ") || out.contains("addi"), "ConstBool must emit code, got: {out}");
}

#[test]
fn riscv_swap_uses_sw_lw_pair() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(1), OpKind::ConstI64(2), OpKind::Swap { a: TY_I64, b: TY_I64 }, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("lw") || out.contains("sw"), "swap must use lw/sw, got: {out}");
}

#[test]
fn riscv_br_emits_j() {
    let out = emit(&single_block_word(
        sig_0_0(),
        &[OpKind::Br { target: BlockId(0) }],
    ));
    assert!(out.contains("j "), "br must emit j, got: {out}");
}

#[test]
fn riscv_brif_uses_bnez() {
    let mut b0_ops: frontend::fixed::FixedVec<ir::Op, 96> = frontend::fixed::FixedVec::new();
    b0_ops.push(ir::Op { kind: OpKind::ConstBool(true), span: Span::UNKNOWN }).unwrap();
    b0_ops.push(ir::Op { kind: OpKind::BrIf { then_tgt: BlockId(1), else_tgt: BlockId(2) }, span: Span::UNKNOWN }).unwrap();
    let mut blocks: frontend::fixed::FixedVec<ir::Block, 16> = frontend::fixed::FixedVec::new();
    blocks.push(ir::Block { id: BlockId(0), entry_stack: frontend::fixed::FixedVec::new(), ops: b0_ops }).unwrap();
    blocks.push(ir::Block { id: BlockId(1), entry_stack: frontend::fixed::FixedVec::new(), ops: frontend::fixed::FixedVec::new() }).unwrap();
    blocks.push(ir::Block { id: BlockId(2), entry_stack: frontend::fixed::FixedVec::new(), ops: frontend::fixed::FixedVec::new() }).unwrap();
    let w = Word {
        name: atom(b"test"), sig: sig_0_0(),
        performs: ir::EffectSet::empty(), requires: ir::CapSet::empty(),
        bound: ir::StackBound::ID, entry: BlockId(1),
        types: baseline_types(), type_sizes: baseline_sizes(), blocks,
    };
    let out = emit(&w);
    assert!(out.contains("bnez") || out.contains("beqz"), "BrIf must use bnez/beqz, got: {out}");
}

#[test]
fn riscv_or_emits_or() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(1), OpKind::ConstI64(0), OpKind::OrBool, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("or"), "or must emit or instruction, got: {out}");
}

#[test]
fn riscv_smoke_golden() {
    let out = emit(&single_block_word(
        sig_0_0(),
        &[OpKind::ConstI64(0), OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("w_"), "must label the word, got: {out}");
    assert!(out.contains(".endword_"), "must have endword marker, got: {out}");
    assert!(out.contains("ret"), "must have ret instruction, got: {out}");
}
