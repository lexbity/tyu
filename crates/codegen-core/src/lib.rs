#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod backend;
pub mod emit_mode;
pub mod error;
pub mod isa;
pub mod platform;
pub mod runtime;
pub mod target;

pub use backend::CodegenBackend;
pub use emit_mode::{AsmMode, EmitMode};
pub use error::CodegenError;
pub use isa::IsaEmitter;
pub use platform::PlatformEmitter;
pub use runtime::{ChannelPayloadKind, RuntimeEmitter};
pub use target::{
    AssemblerKind, CallingConv, Endian, OutputFormat, PlatformCapability, QemuExitConvention,
    QemuSpec, Target, TargetSpec,
};
