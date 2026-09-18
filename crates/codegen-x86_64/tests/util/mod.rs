//! Shared test utilities for codegen-x86_64 tests.

use codegen_core::{AsmMode, CodegenError};
use codegen_x86_64::X86_64HostedBackend;
use frontend::{
    fixed::FixedVec,
    parse::{ModuleAst, Parser},
    span::Span,
};
use ir::{Atom, Block, BlockId, CapSet, EffectSet, Op, OpKind, Sig, StackBound, Word, TY_I64};

/// Minimal `Output` that captures bytes in a `Vec`.
pub struct TestOut(Vec<u8>);
impl TestOut {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).unwrap()
    }
}
impl frontend::parse::Output for TestOut {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

pub fn atom(b: &[u8]) -> Atom {
    Atom::new(b).unwrap()
}

pub fn baseline_types() -> FixedVec<Atom, 64> {
    let mut t = FixedVec::new();
    t.push(atom(b"")).unwrap();
    t.push(atom(b"i64")).unwrap();
    t.push(atom(b"bool")).unwrap();
    t.push(atom(b"str")).unwrap();
    t.push(atom(b"ptr")).unwrap();
    t.push(atom(b"ptr_mut")).unwrap();
    t.push(atom(b"mmio")).unwrap();
    t.push(atom(b"MyType")).unwrap(); // 7 — 4-byte custom type
    t
}

pub fn baseline_sizes() -> FixedVec<u32, 64> {
    let mut s = FixedVec::new();
    s.push(0).unwrap();
    s.push(8).unwrap();
    s.push(1).unwrap();
    s.push(8).unwrap();
    s.push(8).unwrap();
    s.push(8).unwrap();
    s.push(8).unwrap();
    s.push(4).unwrap();
    s
}

pub fn empty_module(src: &[u8]) -> ModuleAst {
    Parser::new(src).parse_module_ast().unwrap()
}

pub fn single_block_word(sig: Sig, ops: &[OpKind]) -> Word {
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

pub fn sig_0_0() -> Sig {
    Sig::empty()
}
pub fn sig_0_1(out: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.out_len = 1;
    s.outputs[0] = out;
    s
}
pub fn sig_0_2(a: ir::TypeId, b: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.out_len = 2;
    s.outputs[0] = a;
    s.outputs[1] = b;
    s
}
pub fn sig_1_0(inp: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.in_len = 1;
    s.inputs[0] = inp;
    s
}
pub fn sig_1_1(inp: ir::TypeId, out: ir::TypeId) -> Sig {
    let mut s = Sig::empty();
    s.in_len = 1;
    s.inputs[0] = inp;
    s.out_len = 1;
    s.outputs[0] = out;
    s
}

/// Emit a word and return the output.
pub fn emit(w: &Word) -> String {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = X86_64HostedBackend::new(&mod_ast, b"", &mut out, false, AsmMode::Executable);
    backend.emit_word(w).unwrap();
    out.as_str().to_string()
}

/// Emit a word expecting a `CodegenError`.
pub fn emit_err(w: &Word) -> CodegenError {
    let mod_ast = empty_module(b"module m; end;");
    let mut out = TestOut::new();
    let mut backend = X86_64HostedBackend::new(&mod_ast, b"", &mut out, false, AsmMode::Executable);
    backend.emit_word(w).unwrap_err()
}

/// Run a closure on an 8 MB thread stack.
macro_rules! run_8mb {
    ($body:expr) => {
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(|| $body)
            .unwrap()
            .join()
            .unwrap();
    };
}
pub(crate) use run_8mb;
