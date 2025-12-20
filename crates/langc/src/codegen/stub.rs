#![allow(dead_code)]

use ir as lir;
use crate::codegen::CodegenBackend;

pub struct StubBackend {
    pub target: &'static [u8],
}

impl StubBackend {
    pub fn new(target: &'static [u8]) -> Self {
        Self { target }
    }
}

impl CodegenBackend for StubBackend {
    fn emit_prelude(&mut self) -> Result<(), u32> {
        Err(7990)
    }

    fn emit_word(&mut self, _w: &lir::Word) -> Result<(), u32> {
        Err(7990)
    }

    fn emit_postlude(&mut self) -> Result<(), u32> {
        Err(7990)
    }
}
