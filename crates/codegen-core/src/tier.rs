//! Operation tiering — which backends support which operations.
//!
//! Each backend exports `op_tier(&OpKind) -> Tier` implemented as an
//! exhaustive `match`.  Adding a new `OpKind` variant MUST cause a
//! compile error here, forcing the developer to classify it.

use ir::CmpKind;
use ir::OpKind;

/// Whether a backend supports a given operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tier {
    /// The backend can emit code for this operation.
    Supported,
    /// The backend returns `CodegenError::UnsupportedOp` for this operation.
    Unsupported,
}

/// Classify an `OpKind` for the ARM Thumb backend.
///
/// Exhaustive: every variant is listed.  Adding a new `OpKind` without
/// adding an arm here is a compile error.
pub fn arm_op_tier(op: &OpKind) -> Tier {
    match op {
        OpKind::ConstI64(_) => Tier::Supported,
        OpKind::ConstBool(_) => Tier::Supported,
        OpKind::ConstStr(_) => Tier::Supported,
        OpKind::AddrOf { const_addr: Some(_), .. } => Tier::Supported,
        OpKind::AddrOf { const_addr: None, .. } => Tier::Supported, // returns UnsupportedAddrOf
        OpKind::MmioPlace { .. } => Tier::Supported,
        OpKind::ScopedEnter { .. } => Tier::Supported,
        OpKind::TaskSpawn { .. } => Tier::Supported,
        OpKind::PtrAddConst { .. } => Tier::Supported,
        OpKind::PtrAddIndex { .. } => Tier::Supported,
        OpKind::Dup { .. } => Tier::Supported,
        OpKind::Drop { .. } => Tier::Supported,
        OpKind::Swap { .. } => Tier::Supported,
        OpKind::AddI64 => Tier::Supported,
        OpKind::SubI64 => Tier::Supported,
        OpKind::MulI64 => Tier::Supported,
        OpKind::Cmp { .. } => Tier::Supported,
        OpKind::AndBool => Tier::Supported,
        OpKind::OrBool => Tier::Supported,
        OpKind::NotBool => Tier::Supported,
        OpKind::LocalSet { .. } => Tier::Supported,
        OpKind::LocalGet { .. } => Tier::Supported,
        OpKind::Call { .. } => Tier::Supported,
        OpKind::Br { .. } => Tier::Supported,
        OpKind::BrIf { .. } => Tier::Supported,
        OpKind::Ret => Tier::Supported,
        OpKind::TrapIfFalse { .. } => Tier::Supported,
        OpKind::Load { .. } => Tier::Supported,
        OpKind::Store { .. } => Tier::Supported,
        OpKind::Cast { .. } => Tier::Supported,
        OpKind::Bitcast { .. } => Tier::Supported,
        OpKind::MmioVolLoad { .. } => Tier::Supported,
        OpKind::MmioVolStore { .. } => Tier::Supported,
        OpKind::MmioVolLoadField { .. } => Tier::Supported,
        OpKind::MmioVolStoreField { .. } => Tier::Supported,
        OpKind::CheckSubtype { .. } => Tier::Unsupported,
    }
}

/// Classify an `OpKind` for the RISC-V backend.
///
/// Exhaustive: every variant is listed.
pub fn riscv_op_tier(op: &OpKind) -> Tier {
    match op {
        OpKind::ConstI64(_) => Tier::Supported,
        OpKind::ConstBool(_) => Tier::Supported,
        OpKind::ConstStr(_) => Tier::Supported,
        OpKind::AddrOf { const_addr: Some(_), .. } => Tier::Supported,
        OpKind::AddrOf { const_addr: None, .. } => Tier::Supported,
        OpKind::MmioPlace { .. } => Tier::Supported,
        OpKind::ScopedEnter { .. } => Tier::Supported,
        OpKind::TaskSpawn { .. } => Tier::Supported,
        OpKind::PtrAddConst { .. } => Tier::Supported,
        OpKind::PtrAddIndex { .. } => Tier::Supported,
        OpKind::Dup { .. } => Tier::Supported,
        OpKind::Drop { .. } => Tier::Supported,
        OpKind::Swap { .. } => Tier::Supported,
        OpKind::AddI64 => Tier::Supported,
        OpKind::SubI64 => Tier::Supported,
        OpKind::MulI64 => Tier::Supported,
        OpKind::Cmp { .. } => Tier::Supported,
        OpKind::AndBool => Tier::Supported,
        OpKind::OrBool => Tier::Supported,
        OpKind::NotBool => Tier::Supported,
        OpKind::LocalSet { .. } => Tier::Supported,
        OpKind::LocalGet { .. } => Tier::Supported,
        OpKind::Call { .. } => Tier::Supported,
        OpKind::Br { .. } => Tier::Supported,
        OpKind::BrIf { .. } => Tier::Supported,
        OpKind::Ret => Tier::Supported,
        OpKind::TrapIfFalse { .. } => Tier::Supported,
        OpKind::Load { .. } => Tier::Supported,
        OpKind::Store { .. } => Tier::Supported,
        OpKind::Cast { .. } => Tier::Supported,
        OpKind::Bitcast { .. } => Tier::Supported,
        OpKind::MmioVolLoad { .. } => Tier::Supported,
        OpKind::MmioVolStore { .. } => Tier::Supported,
        OpKind::MmioVolLoadField { .. } => Tier::Supported,
        OpKind::MmioVolStoreField { .. } => Tier::Supported,
        OpKind::CheckSubtype { .. } => Tier::Unsupported,
    }
}
