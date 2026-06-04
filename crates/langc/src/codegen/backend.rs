// The CodegenBackend trait and its associated error type are defined in the
// codegen-core crate, which is the shared interface library for all target
// backend crates. Re-exported here for use within langc.
pub use codegen_core::{CodegenBackend, CodegenError};

use codegen_arm::ArmThumbBackend;
use codegen_riscv::RiscVBackend;
use codegen_x86_64::X86_64HostedBackend;
use ir as lir;

/// Enum dispatch wrapper: allows using any backend through a single
/// `CodegenBackend` impl without heap allocation.
pub enum Backend<'a> {
    X86(X86_64HostedBackend<'a>),
    Arm(ArmThumbBackend<'a>),
    RiscV(RiscVBackend<'a>),
}

impl<'a> CodegenBackend for Backend<'a> {
    fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        match self {
            Backend::X86(b) => b.emit_prelude(),
            Backend::Arm(b) => b.emit_prelude(),
            Backend::RiscV(b) => b.emit_prelude(),
        }
    }

    fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        match self {
            Backend::X86(b) => b.emit_word(w),
            Backend::Arm(b) => b.emit_word(w),
            Backend::RiscV(b) => b.emit_word(w),
        }
    }

    fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        match self {
            Backend::X86(b) => b.emit_postlude(),
            Backend::Arm(b) => b.emit_postlude(),
            Backend::RiscV(b) => b.emit_postlude(),
        }
    }

    fn emit_extern_word(&mut self, name: &[u8]) -> Result<(), CodegenError> {
        match self {
            Backend::X86(b) => CodegenBackend::emit_extern_word(b, name),
            Backend::Arm(b) => CodegenBackend::emit_extern_word(b, name),
            Backend::RiscV(b) => CodegenBackend::emit_extern_word(b, name),
        }
    }

    fn set_expected_abi_hash(&mut self, hash: u64) {
        match self {
            Backend::X86(b) => b.set_expected_abi_hash(hash),
            Backend::Arm(b) => b.set_expected_abi_hash(hash),
            Backend::RiscV(b) => b.set_expected_abi_hash(hash),
        }
    }
}
