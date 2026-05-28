// The CodegenBackend trait and its associated error type are defined in the
// codegen-core crate, which is the shared interface library for all target
// backend crates. Re-exported here for use within langc.
pub use codegen_core::{CodegenBackend, CodegenError};
