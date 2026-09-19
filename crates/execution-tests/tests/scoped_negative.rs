//! Negative fixtures for D-13 type-class dispatch (Phase P2).
//!
//! The G-5 bug class this slice retires: `ScopedEnter` lowering dispatched on
//! type-name bytes and, when the name matched nothing, emitted *no code* and
//! returned `Ok(())` — silent wrong code in the allocation path. After the
//! fix, an unmatched class is a loud `CodegenError::UnsupportedOp`.
//!
//! The semantics never produce a `ScopedEnter` with an unclassifiable type
//! (every scoped enter is a slice or a region reference), so these fixtures
//! hand-build the IR word — the same artifact a mutated IR would carry — and
//! assert each backend rejects it with `UnsupportedOp { ScopedEnter }`, not a
//! trap. One fixture per triple.

use codegen_core::{AsmMode, CodegenError};
use frontend::{
    fixed::FixedVec,
    parse::{ModuleAst, Output, Parser},
    span::Span,
};
use ir::{Atom, Block, BlockId, CapSet, EffectSet, Op, OpKind, Sig, StackBound, TypeId, Word};

struct TestOut(Vec<u8>);
impl TestOut {
    fn new() -> Self {
        Self(Vec::new())
    }
}
impl Output for TestOut {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

fn atom(bytes: &[u8]) -> Atom {
    Atom::new(bytes).unwrap()
}

/// A word whose only ScopedEnter carries an `Other`-class type (`Percent`).
/// `Percent` is not a slice or region reference, so D-13 requires every
/// backend to reject it with `UnsupportedOp` — never emit nothing.
fn word_with_other_class_scoped_enter() -> Word {
    let mut types: FixedVec<Atom, 64> = FixedVec::new();
    let mut type_sizes: FixedVec<u32, 64> = FixedVec::new();
    let mut type_classes: FixedVec<ir::TypeClass, 64> = FixedVec::new();
    let mut push_ty = |name: &[u8], size: u32| {
        let atom = atom(name);
        types.push(atom).unwrap();
        type_sizes.push(size).unwrap();
        type_classes.push(ir::TypeClass::class_of(name)).unwrap();
    };
    push_ty(b"", 0);
    push_ty(b"i64", 8);
    push_ty(b"bool", 1);
    push_ty(b"Percent", 8); // Other class

    let scoped_ty = TypeId(3);

    let mut sig = Sig::empty();
    sig.in_len = 0;
    sig.out_len = 1;
    sig.outputs[0] = scoped_ty;

    let mut ops: FixedVec<Op, 96> = FixedVec::new();
    ops.push(Op {
        kind: OpKind::ScopedEnter {
            ty: scoped_ty,
            len: 16,
        },
        span: Span::UNKNOWN,
    })
    .unwrap();
    ops.push(Op {
        kind: OpKind::Ret,
        span: Span::UNKNOWN,
    })
    .unwrap();

    Word {
        name: atom(b"scoped_other"),
        sig,
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
        entry: BlockId(0),
        types,
        type_sizes,
        type_classes,
        apertures: frontend::fixed::FixedVec::new(),
        subtype_bases: FixedVec::new(),
        blocks: {
            let mut blocks: FixedVec<Block, 16> = FixedVec::new();
            blocks
                .push(Block {
                    id: BlockId(0),
                    entry_stack: FixedVec::new(),
                    ops,
                })
                .unwrap();
            blocks
        },
    }
}

fn empty_module(src: &[u8]) -> ModuleAst {
    Parser::new(src).parse_module_ast().unwrap()
}

/// Run a closure on an 8 MB thread stack.
///
/// `Word` and the codegen backends are large inline `FixedVec`-backed
/// aggregates; a body that holds a `Word` plus backend instances overflows
/// the 2 MB test-thread default. Mirrors `run_8mb!` in the codegen crates'
/// tests (crates/codegen-x86_64/tests/util/mod.rs).
fn run_8mb(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn assert_unsupported_scoped_enter(label: &str, emit: impl FnOnce(&Word) -> Result<(), CodegenError>) {
    let w = word_with_other_class_scoped_enter();
    let err = emit(&w).expect_err("D-13: ScopedEnter on Other class must fail codegen");
    match err {
        CodegenError::UnsupportedOp { op_name } => {
            assert_eq!(op_name, b"ScopedEnter", "{label}: wrong op name");
        }
        other => panic!("{label}: expected UnsupportedOp(ScopedEnter), got {other:?}"),
    }
}

#[test]
fn arm_rejects_scoped_enter_on_other_class() {
    run_8mb(|| {
        assert_unsupported_scoped_enter("armv7m", |w| {
            let mod_ast = empty_module(b"module m; end;");
            let mut out = TestOut::new();
            let mut backend = codegen_arm::ArmThumbBackend::new(
                &mod_ast,
                b"",
                &mut out,
                false,
                AsmMode::Executable,
            );
            backend.emit_word(w)
        });
    });
}

#[test]
fn riscv_rejects_scoped_enter_on_other_class() {
    run_8mb(|| {
        assert_unsupported_scoped_enter("riscv32", |w| {
            let mod_ast = empty_module(b"module m; end;");
            let mut out = TestOut::new();
            let mut backend = codegen_riscv::RiscVBackend::new(
                &mod_ast,
                b"",
                &mut out,
                false,
                AsmMode::Executable,
            );
            backend.emit_word(w)
        });
    });
}

#[test]
fn x86_64_rejects_scoped_enter_on_other_class() {
    run_8mb(|| {
        assert_unsupported_scoped_enter("x86_64", |w| {
            let mod_ast = empty_module(b"module m; end;");
            let mut out = TestOut::new();
            let mut backend = codegen_x86_64::X86_64HostedBackend::new(
                &mod_ast,
                b"",
                &mut out,
                false,
                AsmMode::Executable,
            );
            backend.emit_word(w)
        });
    });
}

#[test]
fn slice_and_region_ref_classes_still_emit() {
    // Control: the D-13 dispatch must not have broken the legitimate paths.
    // Build a Slice(u8) scoped word and assert it emits (no UnsupportedOp).
    run_8mb(|| {
        let mut types: FixedVec<Atom, 64> = FixedVec::new();
    let mut type_sizes: FixedVec<u32, 64> = FixedVec::new();
    let mut type_classes: FixedVec<ir::TypeClass, 64> = FixedVec::new();
    let mut push_ty = |name: &[u8], size: u32| {
        let atom = atom(name);
        types.push(atom).unwrap();
        type_sizes.push(size).unwrap();
        type_classes.push(ir::TypeClass::class_of(name)).unwrap();
    };
    push_ty(b"", 0);
    push_ty(b"i64", 8);
    push_ty(b"Slice(u8)", 16);

    let slice_ty = TypeId(2);
    let mut sig = Sig::empty();
    sig.in_len = 0;
    sig.out_len = 1;
    sig.outputs[0] = slice_ty;

    let mut ops: FixedVec<Op, 96> = FixedVec::new();
    ops.push(Op {
        kind: OpKind::ScopedEnter {
            ty: slice_ty,
            len: 16,
        },
        span: Span::UNKNOWN,
    })
    .unwrap();
    ops.push(Op {
        kind: OpKind::Ret,
        span: Span::UNKNOWN,
    })
    .unwrap();

    let w = Word {
        name: atom(b"scoped_slice"),
        sig,
        performs: EffectSet::empty(),
        requires: CapSet::empty(),
        bound: StackBound::ID,
        entry: BlockId(0),
        types,
        type_sizes,
        type_classes,
        apertures: frontend::fixed::FixedVec::new(),
        subtype_bases: FixedVec::new(),
        blocks: {
            let mut blocks: FixedVec<Block, 16> = FixedVec::new();
            blocks
                .push(Block {
                    id: BlockId(0),
                    entry_stack: FixedVec::new(),
                    ops,
                })
                .unwrap();
            blocks
        },
    };

    for label in ["armv7m", "riscv32", "x86_64"] {
        let result = match label {
            "armv7m" => {
                let mod_ast = empty_module(b"module m; end;");
                let mut out = TestOut::new();
                let mut backend = codegen_arm::ArmThumbBackend::new(
                    &mod_ast,
                    b"",
                    &mut out,
                    false,
                    AsmMode::Executable,
                );
                backend.emit_word(&w)
            }
            "riscv32" => {
                let mod_ast = empty_module(b"module m; end;");
                let mut out = TestOut::new();
                let mut backend = codegen_riscv::RiscVBackend::new(
                    &mod_ast,
                    b"",
                    &mut out,
                    false,
                    AsmMode::Executable,
                );
                backend.emit_word(&w)
            }
            _ => {
                let mod_ast = empty_module(b"module m; end;");
                let mut out = TestOut::new();
                let mut backend = codegen_x86_64::X86_64HostedBackend::new(
                    &mod_ast,
                    b"",
                    &mut out,
                    false,
                    AsmMode::Executable,
                );
                backend.emit_word(&w)
            }
        };
        assert!(
            result.is_ok(),
            "{label}: Slice-class ScopedEnter must still emit, got {result:?}"
        );
    }
    });
}