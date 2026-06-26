//! Operation support classification — which backends support which operations.
//!
//! Each backend exports `op_tier(&OpKind) -> OpSupport` implemented as an
//! exhaustive `match`.  Adding a new `OpKind` variant MUST cause a
//! compile error here, forcing the developer to classify it.

use ir::OpKind;

/// Whether a backend supports a given operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpSupport {
    /// The backend can emit code for this operation.
    Supported,
    /// The backend returns `CodegenError::UnsupportedOp` for this operation.
    Unsupported,
}

/// Classify an `OpKind` for the ARM Thumb backend.
///
/// Exhaustive: every variant is listed.  Adding a new `OpKind` without
/// adding an arm here is a compile error.
pub fn arm_op_tier(op: &OpKind) -> OpSupport {
    match op {
        OpKind::ConstI64(_) => OpSupport::Supported,
        OpKind::ConstBool(_) => OpSupport::Supported,
        OpKind::ConstStr(_) => OpSupport::Supported,
        OpKind::AddrOf {
            const_addr: Some(_),
            ..
        } => OpSupport::Supported,
        OpKind::AddrOf {
            const_addr: None, ..
        } => OpSupport::Supported, // returns UnsupportedAddrOf
        OpKind::MmioPlace { .. } => OpSupport::Supported,
        OpKind::ScopedEnter { .. } => OpSupport::Supported,
        OpKind::TaskSpawn { .. } => OpSupport::Supported,
        OpKind::PtrAddConst { .. } => OpSupport::Supported,
        OpKind::PtrAddIndex { .. } => OpSupport::Supported,
        OpKind::Dup { .. } => OpSupport::Supported,
        OpKind::Drop { .. } => OpSupport::Supported,
        OpKind::Swap { .. } => OpSupport::Supported,
        OpKind::AddI64 => OpSupport::Supported,
        OpKind::SubI64 => OpSupport::Supported,
        OpKind::MulI64 => OpSupport::Supported,
        OpKind::Cmp { .. } => OpSupport::Supported,
        OpKind::AndBool => OpSupport::Supported,
        OpKind::OrBool => OpSupport::Supported,
        OpKind::NotBool => OpSupport::Supported,
        OpKind::InterruptDisable => OpSupport::Supported,
        OpKind::InterruptEnable => OpSupport::Supported,
        OpKind::LocalSet { .. } => OpSupport::Supported,
        OpKind::LocalGet { .. } => OpSupport::Supported,
        OpKind::Call { .. } => OpSupport::Supported,
        OpKind::Br { .. } => OpSupport::Supported,
        OpKind::BrIf { .. } => OpSupport::Supported,
        OpKind::Ret => OpSupport::Supported,
        OpKind::TrapIfFalse { .. } => OpSupport::Supported,
        OpKind::Load { .. } => OpSupport::Supported,
        OpKind::Store { .. } => OpSupport::Supported,
        OpKind::Cast { .. } => OpSupport::Supported,
        OpKind::Bitcast { .. } => OpSupport::Supported,
        OpKind::MmioVolLoad { .. } => OpSupport::Supported,
        OpKind::MmioVolStore { .. } => OpSupport::Supported,
        OpKind::MmioVolLoadField { .. } => OpSupport::Supported,
        OpKind::MmioVolStoreField { .. } => OpSupport::Supported,
        OpKind::CheckSubtype { .. } => OpSupport::Unsupported,
    }
}

/// Classify an `OpKind` for the RISC-V backend.
///
/// Exhaustive: every variant is listed.
pub fn riscv_op_tier(op: &OpKind) -> OpSupport {
    match op {
        OpKind::ConstI64(_) => OpSupport::Supported,
        OpKind::ConstBool(_) => OpSupport::Supported,
        OpKind::ConstStr(_) => OpSupport::Supported,
        OpKind::AddrOf {
            const_addr: Some(_),
            ..
        } => OpSupport::Supported,
        OpKind::AddrOf {
            const_addr: None, ..
        } => OpSupport::Supported,
        OpKind::MmioPlace { .. } => OpSupport::Supported,
        OpKind::ScopedEnter { .. } => OpSupport::Supported,
        OpKind::TaskSpawn { .. } => OpSupport::Supported,
        OpKind::PtrAddConst { .. } => OpSupport::Supported,
        OpKind::PtrAddIndex { .. } => OpSupport::Supported,
        OpKind::Dup { .. } => OpSupport::Supported,
        OpKind::Drop { .. } => OpSupport::Supported,
        OpKind::Swap { .. } => OpSupport::Supported,
        OpKind::AddI64 => OpSupport::Supported,
        OpKind::SubI64 => OpSupport::Supported,
        OpKind::MulI64 => OpSupport::Supported,
        OpKind::Cmp { .. } => OpSupport::Supported,
        OpKind::AndBool => OpSupport::Supported,
        OpKind::OrBool => OpSupport::Supported,
        OpKind::NotBool => OpSupport::Supported,
        OpKind::InterruptDisable => OpSupport::Supported,
        OpKind::InterruptEnable => OpSupport::Supported,
        OpKind::LocalSet { .. } => OpSupport::Supported,
        OpKind::LocalGet { .. } => OpSupport::Supported,
        OpKind::Call { .. } => OpSupport::Supported,
        OpKind::Br { .. } => OpSupport::Supported,
        OpKind::BrIf { .. } => OpSupport::Supported,
        OpKind::Ret => OpSupport::Supported,
        OpKind::TrapIfFalse { .. } => OpSupport::Supported,
        OpKind::Load { .. } => OpSupport::Supported,
        OpKind::Store { .. } => OpSupport::Supported,
        OpKind::Cast { .. } => OpSupport::Supported,
        OpKind::Bitcast { .. } => OpSupport::Supported,
        OpKind::MmioVolLoad { .. } => OpSupport::Supported,
        OpKind::MmioVolStore { .. } => OpSupport::Supported,
        OpKind::MmioVolLoadField { .. } => OpSupport::Supported,
        OpKind::MmioVolStoreField { .. } => OpSupport::Supported,
        OpKind::CheckSubtype { .. } => OpSupport::Unsupported,
    }
}
