#![no_std]
#![forbid(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
//! Shared target descriptions and backend traits for Tyu code generation.
//!
//! Key entry points include `Target`, `TargetSpec`, `AsmMode`,
//! `AssemblerKind`, `FeatureSet`, `CodegenBackend`, and `CodegenError`.

extern crate alloc;

pub mod backend;
pub mod compiled_desc;
pub mod emit_mode;
pub mod error;
pub mod strategy;
pub mod strings;
pub mod target;
pub mod tier;

pub use backend::CodegenBackend;
pub use compiled_desc::{
    CompiledDescError, CompiledDescriptor, CompiledDevice, CompiledRegister,
    COMPILED_DESC_DEVICE_CAP, COMPILED_DESC_MAX_BYTES, COMPILED_DESC_REGISTER_CAP,
    COMPILED_DESC_WINDOW_CAP, REG_ACCESS_RO, REG_ACCESS_RW, REG_ACCESS_WO, REG_BARRIER_AFTER,
    REG_BARRIER_BEFORE, REG_BARRIER_BOTH, REG_BARRIER_NONE, REG_READ_EFFECTFUL, REG_READ_PLAIN,
    REG_WRITE_PLAIN, REG_WRITE_W1C, REG_WRITE_W1S, decode_compiled_desc, encode_compiled_desc,
    validate_compiled_desc,
};
pub use emit_mode::{AsmMode, EmitMode};
pub use error::CodegenError;
pub use target::{
    AssemblerKind, CallingConv, Endian, Feature, FeatureSet, InterruptSource, MmioScratch,
    MmioWindowKind, MmioWindowSpec, OutputFormat, PlatformCapability, QemuExitConvention,
    QemuSpec, ScratchBacking, Target, TargetSpec,
};

/// Monotonically-increasing revision counter for the codegen + IR format.
///
/// **Bump this on any change that affects compiled output:**
/// - New/changed IR opcode encoding
/// - Modified target spec fields (slot_bytes, pointer_bits, etc.)
/// - Changed `.lmod` header layout or section offsets
/// - Modified codegen backend emit logic
///
/// The build cache incorporates this value so that a bumped `CODEGEN_REV`
/// automatically invalidates all cached artifacts, even when source files
/// and the `langc` binary are unchanged.
pub const CODEGEN_REV: u64 = 2;
