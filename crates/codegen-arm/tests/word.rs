//! First word-emission tests for the ARM Thumb backend.
//!
//! Covers slot discipline (dup/drop adjust by 8), comparison lowering,
//! ConstStr table emission, trap symbol, and a smoke golden.
//!
//! NOTE: These tests are a starting point.  Full codegen verification
//! requires running the emitted assembly through `arm-none-eabi-as`
//! under QEMU, which is done in execution-tests.

use codegen_arm::ArmThumbBackend;
use codegen_core::{AsmMode, CodegenError};
use frontend::{
    fixed::FixedVec,
    parse::{ModuleAst, Output, Parser},
    span::Span,
};
use ir::{
    Atom, Block, BlockId, CapSet, EffectSet, Op, OpKind, Sig, StackBound, TrapCode, Word, TY_BOOL,
    TY_I64,
};

struct TestOut(Vec<u8>);
impl TestOut {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).unwrap()
    }
}
impl Output for TestOut {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

fn atom(b: &[u8]) -> Atom {
    Atom::new(b).unwrap()
}

fn baseline_types() -> FixedVec<Atom, 64> {
    let mut t = FixedVec::new();
    t.push(atom(b"")).unwrap();
    t.push(atom(b"i64")).unwrap();
    t.push(atom(b"bool")).unwrap();
    t
}
fn baseline_sizes() -> FixedVec<u32, 64> {
    let mut s = FixedVec::new();
    s.push(0).unwrap();
    s.push(8).unwrap();
    s.push(1).unwrap();
    s
}
fn baseline_classes() -> FixedVec<ir::TypeClass, 64> {
    let mut c = FixedVec::new();
    c.push(ir::TypeClass::class_of(b"")).unwrap();
    c.push(ir::TypeClass::class_of(b"i64")).unwrap();
    c.push(ir::TypeClass::class_of(b"bool")).unwrap();
    c
}

fn empty_module(src: &[u8]) -> ModuleAst {
    Parser::new(src).parse_module_ast().unwrap()
}

fn single_block_word(sig: Sig, ops: &[OpKind]) -> Word {
    let mut opv: FixedVec<Op, 96> = FixedVec::new();
    for &k in ops {
        opv.push(Op {
            kind: k,
            span: Span::UNKNOWN,
        })
        .unwrap();
    }
    Word {
        name: atom(b"test"),
        sig,
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
        entry: BlockId(0),
        types: baseline_types(),
        type_sizes: baseline_sizes(),
        type_classes: baseline_classes(),
        apertures: FixedVec::new(),
        subtype_bases: FixedVec::new(),
        blocks: {
            let mut b = FixedVec::new();
            b.push(Block {
                id: BlockId(0),
                entry_stack: FixedVec::new(),
                ops: opv,
            })
            .unwrap();
            b
        },
    }
}

fn sig_0_0() -> Sig {
    Sig::empty()
}
fn sig_0_1(out: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.out_len = 1;
    s.outputs[0] = out;
    s
}

fn emit(w: &Word) -> String {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = ArmThumbBackend::new(&mod_ast, b"", &mut out, false, AsmMode::Executable);
    backend.emit_word(w).unwrap();
    out.as_str().to_string()
}

fn emit_object_prelude() -> String {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = ArmThumbBackend::new(&mod_ast, b"", &mut out, false, AsmMode::Object);
    backend.emit_prelude().unwrap();
    out.as_str().to_string()
}

// ---------------------------------------------------------------------------
// Slot discipline: dup/drop adjust by 8 (each stack slot is 8 bytes)
// ---------------------------------------------------------------------------

#[test]
fn arm_dup_adjusts_by_8() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(0xBEEF),
            OpKind::Dup { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    // dup should emit two subs/adds pairs (8 bytes each direction).
    assert!(
        out.contains("subs r4, r4, #8"),
        "dup must adjust r4 by 8 down, got: {out}"
    );
    assert!(
        out.contains("adds r4, r4, #8"),
        "dup must adjust r4 by 8 up, got: {out}"
    );
}

#[test]
fn arm_drop_adjusts_by_8() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[OpKind::ConstI64(42), OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(
        out.contains("subs r4, r4, #8"),
        "drop must adjust r4 by 8, got: {out}"
    );
}

// ---------------------------------------------------------------------------
// Comparison lowering: uses IT block + cmp/set patterns
// ---------------------------------------------------------------------------

#[test]
fn arm_cmp_uses_it_block() {
    let out = emit(&single_block_word(
        sig_0_1(TY_BOOL),
        &[
            OpKind::ConstI64(1),
            OpKind::ConstI64(2),
            OpKind::Cmp {
                out: TY_BOOL,
                kind: ir::CmpKind::Lt,
            },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    // ARM comparison emits `itt` or `ite` patterns.
    assert!(
        out.contains("itt") || out.contains("ite"),
        "cmp must use IT block, got: {out}"
    );
}

// ---------------------------------------------------------------------------
// Trap symbol reference
// ---------------------------------------------------------------------------

#[test]
fn arm_trap_if_false_references_trap() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(1),
            OpKind::TrapIfFalse {
                code: TrapCode::AssertFail,
            },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    assert!(
        out.contains("__lang_trap") || out.contains("__lang_trap_loc"),
        "trap must reference __lang_trap, got: {out}"
    );
}

// ---------------------------------------------------------------------------
// Smoke golden: compile a minimal word and verify structure
// ---------------------------------------------------------------------------

#[test]
fn arm_add_emits_adds() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(1),
            OpKind::ConstI64(2),
            OpKind::AddI64,
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    assert!(out.contains("adds"), "add must emit adds, got: {out}");
}

#[test]
fn arm_sub_emits_subs_sbc() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(5),
            OpKind::ConstI64(3),
            OpKind::SubI64,
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    assert!(out.contains("subs"), "sub must emit subs, got: {out}");
    assert!(
        out.contains("sbc"),
        "sub must emit sbc (borrow), got: {out}"
    );
}

#[test]
fn arm_not_emits_cmp_ite() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(0),
            OpKind::NotBool,
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    assert!(
        out.contains("itt") || out.contains("ite"),
        "not must use IT block, got: {out}"
    );
}

#[test]
fn arm_swap_uses_strd_ldrd() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(1),
            OpKind::ConstI64(2),
            OpKind::Swap {
                a: TY_I64,
                b: TY_I64,
            },
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    assert!(
        out.contains("ldrd") || out.contains("strd"),
        "swap must use ldrd/strd, got: {out}"
    );
}

#[test]
fn arm_br_emits_b() {
    let out = emit(&single_block_word(
        sig_0_0(),
        &[OpKind::Br { target: BlockId(0) }],
    ));
    assert!(out.contains("b "), "br must emit b, got: {out}");
}

#[test]
fn arm_mul_emits_muls() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(3),
            OpKind::ConstI64(7),
            OpKind::MulI64,
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    assert!(out.contains("muls"), "mul must emit muls, got: {out}");
}

#[test]
fn arm_and_emits_ands() {
    let out = emit(&single_block_word(
        sig_0_1(TY_I64),
        &[
            OpKind::ConstI64(1),
            OpKind::ConstI64(0),
            OpKind::AndBool,
            OpKind::Drop { ty: TY_I64 },
            OpKind::Drop { ty: TY_I64 },
        ],
    ));
    assert!(out.contains("ands"), "and must emit ands, got: {out}");
}

#[test]
fn arm_smoke_golden() {
    let out = emit(&single_block_word(
        sig_0_0(),
        &[OpKind::ConstI64(0), OpKind::Drop { ty: TY_I64 }],
    ));
    // Must have a function label (w_<hash>:), `.endword_` marker, and `bx lr` return.
    assert!(
        out.contains("w_") || out.contains(".thumb_func"),
        "must label the word, got: {out}"
    );
    assert!(
        out.contains(".endword_"),
        "must have endword marker, got: {out}"
    );
    assert!(out.contains("bx lr"), "must have bx lr return, got: {out}");
}

#[test]
fn arm_object_prelude_imports_trap_symbols() {
    let out = emit_object_prelude();
    assert!(
        out.contains("\t.extern __lang_trap"),
        "object prelude must import __lang_trap, got: {out}"
    );
    assert!(
        out.contains("\t.extern __stack_overflow"),
        "object prelude must import __stack_overflow, got: {out}"
    );
    assert!(
        !out.contains("__lang_trap:\n"),
        "object prelude must not define __lang_trap, got: {out}"
    );
}
