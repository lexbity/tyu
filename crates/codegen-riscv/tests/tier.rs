//! Exhaustive operation tier test for the RISC-V backend.

use codegen_core::tier::{riscv_op_tier, Tier};
use frontend::span::Span;
use ir::OpKind;


fn all_ops() -> Vec<(OpKind, Tier)> {
    use ir::{BlockId, CmpKind, TrapCode};
    vec![
        (OpKind::ConstI64(0), Tier::Supported),
        (OpKind::ConstBool(true), Tier::Supported),
        (OpKind::ConstStr(Span::UNKNOWN), Tier::Supported),
        (OpKind::AddrOf { place: ir::Atom::new(b"x").unwrap(), mutable: false, const_addr: Some(0) }, Tier::Supported),
        (OpKind::AddrOf { place: ir::Atom::new(b"x").unwrap(), mutable: false, const_addr: None }, Tier::Supported),
        (OpKind::MmioPlace { place: ir::Atom::new(b"r").unwrap(), addr: 0x1000 }, Tier::Supported),
        (OpKind::ScopedEnter { ty: ir::TypeId(1), len: 8 }, Tier::Supported),
        (OpKind::TaskSpawn { name: ir::Atom::new(b"f").unwrap(), task_ty: ir::TypeId(1) }, Tier::Supported),
        (OpKind::PtrAddConst { ty: ir::TypeId(1), offset: 8 }, Tier::Supported),
        (OpKind::PtrAddIndex { ty: ir::TypeId(1), scale: 8 }, Tier::Supported),
        (OpKind::Dup { ty: ir::TypeId(1) }, Tier::Supported),
        (OpKind::Drop { ty: ir::TypeId(1) }, Tier::Supported),
        (OpKind::Swap { a: ir::TypeId(1), b: ir::TypeId(1) }, Tier::Supported),
        (OpKind::AddI64, Tier::Supported),
        (OpKind::SubI64, Tier::Supported),
        (OpKind::MulI64, Tier::Supported),
        (OpKind::Cmp { out: ir::TypeId(1), kind: CmpKind::Lt }, Tier::Supported),
        (OpKind::AndBool, Tier::Supported),
        (OpKind::OrBool, Tier::Supported),
        (OpKind::NotBool, Tier::Supported),
        (OpKind::LocalSet { slot: 0, ty: ir::TypeId(1) }, Tier::Supported),
        (OpKind::LocalGet { slot: 0, ty: ir::TypeId(1) }, Tier::Supported),
        (OpKind::Call { name: ir::Atom::new(b"f").unwrap(), sig: ir::Sig::empty(), performs: ir::EffectSet::empty(), requires: ir::CapSet::empty(), bound: ir::StackBound::ID }, Tier::Supported),
        (OpKind::Br { target: BlockId(0) }, Tier::Supported),
        (OpKind::BrIf { then_tgt: BlockId(1), else_tgt: BlockId(2) }, Tier::Supported),
        (OpKind::Ret, Tier::Supported),
        (OpKind::TrapIfFalse { code: TrapCode::AssertFail }, Tier::Supported),
        (OpKind::Load { ty: ir::TypeId(1) }, Tier::Supported),
        (OpKind::Store { ty: ir::TypeId(1) }, Tier::Supported),
        (OpKind::Cast { from: ir::TypeId(1), to: ir::TypeId(1) }, Tier::Supported),
        (OpKind::Bitcast { from: ir::TypeId(1), to: ir::TypeId(1) }, Tier::Supported),
        (OpKind::MmioVolLoad { ty: ir::TypeId(1), place: ir::Atom::new(b"r").unwrap() }, Tier::Supported),
        (OpKind::MmioVolStore { ty: ir::TypeId(1), place: ir::Atom::new(b"r").unwrap(), access: ir::MmioAccess::Rw }, Tier::Supported),
        (OpKind::MmioVolLoadField { reg_ty: ir::TypeId(1), field_ty: ir::TypeId(1), place: ir::Atom::new(b"r").unwrap(), mask: 0xFF, shift: 0 }, Tier::Supported),
        (OpKind::MmioVolStoreField { reg_ty: ir::TypeId(1), field_ty: ir::TypeId(1), place: ir::Atom::new(b"r").unwrap(), mask: 0xFF, shift: 0 }, Tier::Supported),
        (OpKind::CheckSubtype { ty: ir::TypeId(1) }, Tier::Unsupported),
    ]
}

#[test]
fn riscv_every_operation_is_classified() {
    for (op, expected) in all_ops() {
        let actual = riscv_op_tier(&op);
        assert_eq!(
            actual, expected,
            "RISC-V tier mismatch for {op:?}: expected {expected:?}, got {actual:?}"
        );
    }
}
