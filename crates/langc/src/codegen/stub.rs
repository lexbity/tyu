#![allow(dead_code)]

use ir as lir;
use crate::codegen::{CodegenBackend, CodegenError};

/// Placeholder backend for target triples that have not yet been implemented.
///
/// All methods return `CodegenError::UnsupportedEmitMode`. This backend is
/// selected by the driver when the requested target has no concrete backend
/// crate registered, producing a clean diagnostic rather than a panic.
pub struct StubBackend {
    pub target: &'static [u8],
}

impl StubBackend {
    pub fn new(target: &'static [u8]) -> Self {
        Self { target }
    }
}

impl CodegenBackend for StubBackend {
    fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        Err(CodegenError::UnsupportedEmitMode)
    }

    fn emit_word(&mut self, _w: &lir::Word) -> Result<(), CodegenError> {
        Err(CodegenError::UnsupportedEmitMode)
    }

    fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        Err(CodegenError::UnsupportedEmitMode)
    }
}
