use frontend::parse::Output;
use frontend::{fixed::FixedVec, span::Span};
use ir::{Atom, Block, BlockId, CapSet, CmpKind, EffectSet, Op, OpKind, Sig, StackBound, TrapCode, Word, TY_BOOL, TY_I64, TY_PTR, TY_PTR_MUT};
use codegen_x86_64::X86_64HostedBackend;
use codegen_core::{AsmMode, CodegenError};

/// Minimal `Output` that captures bytes in a `Vec`.
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
    t.push(atom(b"str")).unwrap();
    t.push(atom(b"ptr")).unwrap();
    t.push(atom(b"ptr_mut")).unwrap();
    t.push(atom(b"mmio")).unwrap();
    t.push(atom(b"MyType")).unwrap(); // 7
    t
}
fn baseline_sizes() -> FixedVec<u32, 64> {
    let mut s = FixedVec::new();
    s.push(0).unwrap(); s.push(8).unwrap(); s.push(1).unwrap();
    s.push(8).unwrap(); s.push(8).unwrap(); s.push(8).unwrap();
    s.push(8).unwrap(); s.push(4).unwrap();
    s
}

fn empty_module(src: &[u8]) -> frontend::parse::ModuleAst {
    frontend::parse::Parser::new(src).parse_module_ast().unwrap()
}

/// Build a minimal `Word` with a single block containing the given ops.
fn single_block_word(sig: Sig, ops: &[OpKind]) -> Word {
    let mut opv: FixedVec<Op, 96> = FixedVec::new();
    for &k in ops {
        opv.push(Op { kind: k, span: Span::UNKNOWN }).unwrap();
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
        blocks: {
            let mut b = FixedVec::new();
            b.push(Block { id: BlockId(0), entry_stack: FixedVec::new(), ops: opv }).unwrap();
            b
        },
    }
}

fn sig_0_0() -> Sig { Sig::empty() }
fn sig_0_1(out: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.out_len = 1; s.outputs[0] = out; s
}
fn sig_0_2(a: ir::TypeId, b: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.out_len = 2; s.outputs[0] = a; s.outputs[1] = b; s
}
fn sig_1_0(inp: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.in_len = 1; s.inputs[0] = inp; s
}
fn sig_1_1(inp: ir::TypeId, out: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.in_len = 1; s.inputs[0] = inp;
    s.out_len = 1; s.outputs[0] = out; s
}

/// Emit a word and return the output.
fn emit(w: &Word) -> String {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = X86_64HostedBackend::new(
        &mod_ast, b"", &mut out, false, AsmMode::Executable,
    );
    backend.emit_word(w).unwrap();
    out.as_str().to_string()
}

/// Emit a word expecting a `CodegenError`.
fn emit_err(w: &Word) -> CodegenError {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = X86_64HostedBackend::new(
        &mod_ast, b"", &mut out, false, AsmMode::Executable,
    );
    backend.emit_word(w).unwrap_err()
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn emit_const_i64() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(42), OpKind::Ret]));
    assert!(out.contains("42") || out.contains("2a"), "got: {out}");
}

#[test]
fn emit_const_bool_true() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstBool(true), OpKind::Ret]));
    assert!(out.contains("1") || out.contains("true"), "got: {out}");
}

#[test]
fn emit_const_bool_false() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstBool(false), OpKind::Ret]));
    assert!(out.contains("0"), "got: {out}");
}

// ---------------------------------------------------------------------------
// Stack ops
// ---------------------------------------------------------------------------

#[test]
fn emit_dup() {
    let out = emit(&single_block_word(sig_0_2(TY_I64, TY_I64), &[OpKind::ConstI64(7), OpKind::Dup { ty: TY_I64 }, OpKind::Ret]));
    assert!(out.contains("push rax") || out.contains("[r15]"), "got: {out}");
}

#[test]
fn emit_drop() {
    // drop with sig ( i64 -- )
    let out = emit(&single_block_word(sig_1_0(TY_I64), &[OpKind::Drop { ty: TY_I64 }]));
    assert!(out.contains("add r15, 8") || out.contains("sub r15"), "got: {out}");
}

#[test]
fn emit_swap() {
    let out = emit(&single_block_word(
        sig_0_2(TY_I64, TY_I64),
        &[OpKind::ConstI64(1), OpKind::ConstI64(2), OpKind::Swap { a: TY_I64, b: TY_I64 }, OpKind::Drop { ty: TY_I64 }, OpKind::Drop { ty: TY_I64 }],
    ));
    assert!(out.contains("xchg") || out.contains("mov"), "got: {out}");
}

// ---------------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------------

#[test]
fn emit_add() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(1), OpKind::ConstI64(2), OpKind::AddI64, OpKind::Ret]));
    assert!(out.contains("add"), "got: {out}");
}

#[test]
fn emit_sub() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(5), OpKind::ConstI64(3), OpKind::SubI64, OpKind::Ret]));
    assert!(out.contains("sub"), "got: {out}");
}

#[test]
fn emit_mul() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(2), OpKind::ConstI64(3), OpKind::MulI64, OpKind::Ret]));
    assert!(out.contains("imul") || out.contains("mul"), "got: {out}");
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

fn compare_op(kind: CmpKind, asm: &str) {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[
        OpKind::ConstI64(1), OpKind::ConstI64(2),
        OpKind::Cmp { out: TY_BOOL, kind },
        OpKind::Ret,
    ]));
    assert!(out.contains(asm), "cmp {kind:?} expected {asm}, got: {out}");
}

#[test]
fn emit_cmp_lt() { compare_op(CmpKind::Lt, "setl"); }
#[test]
fn emit_cmp_le() { compare_op(CmpKind::Le, "setle"); }
#[test]
fn emit_cmp_gt() { compare_op(CmpKind::Gt, "setg"); }
#[test]
fn emit_cmp_ge() { compare_op(CmpKind::Ge, "setge"); }
#[test]
fn emit_cmp_eq() { compare_op(CmpKind::Eq, "sete"); }
#[test]
fn emit_cmp_ne() { compare_op(CmpKind::Ne, "setne"); }

// ---------------------------------------------------------------------------
// Boolean ops
// ---------------------------------------------------------------------------

#[test]
fn emit_and() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(1), OpKind::ConstI64(0), OpKind::AndBool, OpKind::Ret]));
    assert!(out.contains("and"), "got: {out}");
}

#[test]
fn emit_or() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(1), OpKind::ConstI64(0), OpKind::OrBool, OpKind::Ret]));
    assert!(out.contains("or"), "got: {out}");
}

#[test]
fn emit_not() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(0), OpKind::NotBool, OpKind::Ret]));
    // NotBool compares with 0, sets al=1 if equal (false→true, true→false)
    assert!(out.contains("cmp") || out.contains("sete"), "got: {out}");
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn emit_br_if() {
    // block 0: push bool, br_if to block 1 else block 2
    let mut b0_ops: FixedVec<Op, 96> = FixedVec::new();
    b0_ops.push(Op { kind: OpKind::ConstBool(true), span: Span::UNKNOWN }).unwrap();
    b0_ops.push(Op { kind: OpKind::BrIf { then_tgt: BlockId(1), else_tgt: BlockId(2) }, span: Span::UNKNOWN }).unwrap();
    let mut blocks: FixedVec<Block, 16> = FixedVec::new();
    blocks.push(Block { id: BlockId(0), entry_stack: FixedVec::new(), ops: b0_ops }).unwrap();
    blocks.push(Block { id: BlockId(1), entry_stack: FixedVec::new(), ops: FixedVec::new() }).unwrap();
    blocks.push(Block { id: BlockId(2), entry_stack: FixedVec::new(), ops: FixedVec::new() }).unwrap();

    let w = Word {
        name: atom(b"test"), sig: sig_0_0(),
        performs: EffectSet::empty(), requires: CapSet::empty(), bound: StackBound::ID,
        entry: BlockId(1),
        types: baseline_types(), type_sizes: baseline_sizes(), blocks,
    };
    let out = emit(&w);
    assert!(out.contains("j") || out.contains("cmp"), "got: {out}");
}

#[test]
fn emit_ret() {
    let out = emit(&single_block_word(sig_0_0(), &[OpKind::ConstI64(0), OpKind::Drop { ty: TY_I64 }]));
    assert!(out.contains("ret"), "got: {out}");
}

// ---------------------------------------------------------------------------
// Trap
// ---------------------------------------------------------------------------

#[test]
fn emit_trap_if_false() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[
        OpKind::ConstI64(1),         OpKind::TrapIfFalse { code: TrapCode::AssertFail },
        OpKind::Ret,
    ]));
    assert!(out.contains("__lang_trap"), "got: {out}");
}

// ---------------------------------------------------------------------------
// Load / Store
// ---------------------------------------------------------------------------

#[test]
fn emit_load() {
    let w = single_block_word(sig_0_1(TY_I64), &[
        OpKind::AddrOf { place: atom(b"x"), mutable: true, const_addr: Some(0x1000) },
        OpKind::Load { ty: TY_I64 },
        OpKind::Ret,
    ]);
    let out = emit(&w);
    assert!(out.contains("mov"), "got: {out}");
}

#[test]
fn emit_store() {
    let w = single_block_word(sig_0_1(TY_I64), &[
        OpKind::AddrOf { place: atom(b"x"), mutable: true, const_addr: Some(0x1000) },
        OpKind::ConstI64(42),
        OpKind::Store { ty: TY_I64 },
        OpKind::ConstI64(0),
        OpKind::Ret,
    ]);
    let out = emit(&w);
    assert!(out.contains("mov"), "got: {out}");
}

// ---------------------------------------------------------------------------
// PtrAdd
// ---------------------------------------------------------------------------

#[test]
fn emit_ptr_add_const() {
    let w = single_block_word(sig_0_1(TY_I64), &[
        OpKind::AddrOf { place: atom(b"x"), mutable: false, const_addr: Some(0x1000) },
        OpKind::PtrAddConst { ty: TY_PTR, offset: 8 },
        OpKind::Drop { ty: TY_PTR },
        OpKind::ConstI64(0),
        OpKind::Ret,
    ]);
    let out = emit(&w);
    assert!(out.contains("add") || out.contains("lea"), "got: {out}");
}

// ---------------------------------------------------------------------------
// Cast
// ---------------------------------------------------------------------------

#[test]
fn emit_cast() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(1), OpKind::Cast { from: TY_I64, to: TY_I64 }, OpKind::Ret]));
    assert!(out.contains("push") || out.contains("mov"), "got: {out}");
}

#[test]
fn emit_bitcast() {
    let out = emit(&single_block_word(sig_0_1(TY_I64), &[OpKind::ConstI64(1), OpKind::Bitcast { from: TY_I64, to: TY_I64 }, OpKind::Ret]));
    assert!(!out.is_empty(), "got: {out}");
}

// ---------------------------------------------------------------------------
// Unsupported ops produce errors
// ---------------------------------------------------------------------------

#[test]
fn emit_addr_of_unsupported() {
    // Without const_addr, AddrOf is not supported.
    let w = single_block_word(sig_0_1(TY_PTR), &[OpKind::AddrOf { place: atom(b"x"), mutable: false, const_addr: None }, OpKind::Ret]);
    let err = emit_err(&w);
    assert!(matches!(err, CodegenError::UnsupportedAddrOf), "got: {err:?}");
}

#[test]
fn emit_check_subtype_unsupported() {
    let w = single_block_word(sig_0_1(TY_I64), &[
        OpKind::ConstI64(42), OpKind::CheckSubtype { ty: TY_I64 },
        OpKind::Drop { ty: TY_BOOL }, OpKind::Ret,
    ]);
    let err = emit_err(&w);
    assert!(matches!(err, CodegenError::UnsupportedCheckSubtype), "got: {err:?}");
}
