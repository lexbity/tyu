#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod backend;
pub mod emit_mode;
pub mod error;
pub mod strings;
pub mod target;

pub use backend::CodegenBackend;
pub use emit_mode::{AsmMode, EmitMode};
pub use error::CodegenError;
pub use target::{
    AssemblerKind, CallingConv, Endian, Feature, FeatureSet, OutputFormat, PlatformCapability,
    QemuExitConvention, QemuSpec, Target, TargetSpec,
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
pub const CODEGEN_REV: u64 = 1;
