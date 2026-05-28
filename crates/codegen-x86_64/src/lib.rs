#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use frontend::{
    parse::{ModuleAst, Output},
    span::Span,
};
use ir as lir;
use codegen_core::{AsmMode, CodegenBackend, CodegenError};

pub mod channel;
pub mod mmio;
pub mod ophelpers;
pub mod postlude;
pub mod prelude;
pub mod region;
pub mod strings;
pub mod task;
pub(crate) mod util;
pub mod word;

pub struct X86_64HostedBackend<'a> {
    pub module: &'a ModuleAst,
    pub src: &'a [u8],
    pub out: &'a mut dyn Output,
    pub mode: AsmMode,
    pub label_id: u32,
    pub uses_channels: bool,
    pub uses_mmio: bool,
    pub uses_regions: bool,
    pub uses_tasks: bool,
    pub str_len: usize,
    pub str_spans: [Span; 128],
    pub str_ids: [u32; 128],
    pub debug_trap_loc: bool,
    pub cur_word_id: u32,
    pub scoped_base: u32,
    pub scoped_slots: u32,
    pub scoped_next: u32,
}

impl<'a> X86_64HostedBackend<'a> {
    pub fn new(
        module: &'a ModuleAst,
        src: &'a [u8],
        out: &'a mut dyn Output,
        debug_trap_loc: bool,
        mode: AsmMode,
    ) -> Self {
        Self {
            module,
            src,
            out,
            mode,
            label_id: 0,
            uses_channels: false,
            uses_mmio: false,
            uses_regions: false,
            uses_tasks: false,
            str_len: 0,
            str_spans: [Span::UNKNOWN; 128],
            str_ids: [0u32; 128],
            debug_trap_loc,
            cur_word_id: 0,
            scoped_base: 0,
            scoped_slots: 0,
            scoped_next: 0,
        }
    }

    pub fn fresh_label(&mut self) -> u32 {
        let id = self.label_id;
        self.label_id = self.label_id.wrapping_add(1);
        id
    }
}

impl<'a> CodegenBackend for X86_64HostedBackend<'a> {
    fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        X86_64HostedBackend::emit_prelude(self)
    }

    fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        X86_64HostedBackend::emit_word(self, w)
    }

    fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        X86_64HostedBackend::emit_postlude(self)
    }

    fn emit_extern_word(&mut self, name: &[u8]) -> Result<(), CodegenError> {
        X86_64HostedBackend::emit_extern_word(self, name);
        Ok(())
    }
}
