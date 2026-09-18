//! Exhaustive operation support test for the ARM Thumb backend.
//!
//! Every `OpKind` variant is classified as `Supported` or `Unsupported`.
//! Adding a new `OpKind` without classifying it here is a compile error
//! (the match in `codegen_core::tier::arm_op_tier` enforces this).

use codegen_core::tier::{arm_op_tier, OpSupport};
use frontend::span::Span;
use ir::OpKind;

fn all_ops() -> Vec<(OpKind, OpSupport)> {
    use ir::{BlockId, CmpKind, TrapCode};
    vec![
        (OpKind::ConstI64(0), OpSupport::Supported),
        (OpKind::ConstBool(true), OpSupport::Supported),
        (OpKind::ConstStr(Span::UNKNOWN), OpSupport::Supported),
        (
            OpKind::AddrOf {
                place: ir::Atom::new(b"x").unwrap(),
                mutable: false,
                base: ir::AddrOfBase::Mmio { window: 0, offset: 0x1000 },
            },
            OpSupport::Supported,
        ),
        (
            OpKind::AddrOf {
                place: ir::Atom::new(b"x").unwrap(),
                mutable: false,
                base: ir::AddrOfBase::Runtime,
            },
            OpSupport::Supported,
        ),
        (
            OpKind::MmioPlace {
                place: ir::Atom::new(b"r").unwrap(),
                window: 0,
                offset: 0x1000,
            },
            OpSupport::Supported,
        ),
        (
            OpKind::ScopedEnter {
                ty: ir::TypeId(1),
                len: 8,
            },
            OpSupport::Supported,
        ),
        (
            OpKind::TaskSpawn {
                name: ir::Atom::new(b"f").unwrap(),
                task_ty: ir::TypeId(1),
            },
            OpSupport::Supported,
        ),
        (
            OpKind::PtrAddConst {
                ty: ir::TypeId(1),
                offset: 8,
            },
            OpSupport::Supported,
        ),
        (
            OpKind::PtrAddIndex {
                ty: ir::TypeId(1),
                scale: 8,
            },
            OpSupport::Supported,
        ),
        (OpKind::Dup { ty: ir::TypeId(1) }, OpSupport::Supported),
        (OpKind::Drop { ty: ir::TypeId(1) }, OpSupport::Supported),
        (
            OpKind::Swap {
                a: ir::TypeId(1),
                b: ir::TypeId(1),
            },
            OpSupport::Supported,
        ),
        (OpKind::AddI64, OpSupport::Supported),
        (OpKind::SubI64, OpSupport::Supported),
        (OpKind::MulI64, OpSupport::Supported),
        (
            OpKind::Cmp {
                out: ir::TypeId(1),
                kind: CmpKind::Lt,
            },
            OpSupport::Supported,
        ),
        (OpKind::AndBool, OpSupport::Supported),
        (OpKind::OrBool, OpSupport::Supported),
        (OpKind::NotBool, OpSupport::Supported),
        (OpKind::InterruptDisable, OpSupport::Supported),
        (OpKind::InterruptEnable, OpSupport::Supported),
        (
            OpKind::LocalSet {
                slot: 0,
                ty: ir::TypeId(1),
            },
            OpSupport::Supported,
        ),
        (
            OpKind::LocalGet {
                slot: 0,
                ty: ir::TypeId(1),
            },
            OpSupport::Supported,
        ),
        (
            OpKind::Call {
                name: ir::Atom::new(b"f").unwrap(),
                sig: ir::Sig::empty(),
                performs: ir::EffectSet::empty(),
                requires: ir::CapSet::empty(),
                bound: ir::StackBound::ID,
            },
            OpSupport::Supported,
        ),
        (OpKind::Br { target: BlockId(0) }, OpSupport::Supported),
        (
            OpKind::BrIf {
                then_tgt: BlockId(1),
                else_tgt: BlockId(2),
            },
            OpSupport::Supported,
        ),
        (OpKind::Ret, OpSupport::Supported),
        (
            OpKind::TrapIfFalse {
                code: TrapCode::AssertFail,
            },
            OpSupport::Supported,
        ),
        (OpKind::Load { ty: ir::TypeId(1) }, OpSupport::Supported),
        (OpKind::Store { ty: ir::TypeId(1) }, OpSupport::Supported),
        (
            OpKind::Cast {
                from: ir::TypeId(1),
                to: ir::TypeId(1),
            },
            OpSupport::Supported,
        ),
        (
            OpKind::Bitcast {
                from: ir::TypeId(1),
                to: ir::TypeId(1),
            },
            OpSupport::Supported,
        ),
        (
            OpKind::MmioVolLoad {
                ty: ir::TypeId(1),
                place: ir::Atom::new(b"r").unwrap(),
            },
            OpSupport::Supported,
        ),
        (
            OpKind::MmioVolStore {
                ty: ir::TypeId(1),
                place: ir::Atom::new(b"r").unwrap(),
                access: ir::MmioAccess::Rw,
            },
            OpSupport::Supported,
        ),
        (
            OpKind::MmioVolLoadField {
                reg_ty: ir::TypeId(1),
                field_ty: ir::TypeId(1),
                place: ir::Atom::new(b"r").unwrap(),
                mask: 0xFF,
                shift: 0,
            },
            OpSupport::Supported,
        ),
        (
            OpKind::MmioVolStoreField {
                reg_ty: ir::TypeId(1),
                field_ty: ir::TypeId(1),
                place: ir::Atom::new(b"r").unwrap(),
                mask: 0xFF,
                shift: 0,
            },
            OpSupport::Supported,
        ),
        (
            OpKind::CheckSubtype { ty: ir::TypeId(1) },
            OpSupport::Unsupported,
        ),
    ]
}

#[test]
fn arm_every_operation_is_classified() {
    for (op, expected) in all_ops() {
        let actual = arm_op_tier(&op);
        assert_eq!(
            actual, expected,
            "ARM tier mismatch for {op:?}: expected {expected:?}, got {actual:?}"
        );
    }
}
