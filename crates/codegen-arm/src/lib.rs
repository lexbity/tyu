#![no_std]
#![forbid(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
//! ARM Thumb code generation backend for Tyu.
//!
//! The public surface is centered on `ArmThumbBackend`, with support modules
//! in `ophelpers`, `postlude`, `prelude`, and `word`.

use codegen_core::{AsmMode, CodegenBackend, CodegenError};
use frontend::{
    parse::{AttrAst, DeclKind, ModuleAst, Output},
    span::Span,
};
use ir as lir;

pub mod ophelpers;
pub mod postlude;
pub mod prelude;
pub mod word;

/// Metadata collected for an export during word emission.
#[derive(Clone, Copy)]
pub(crate) struct ModInfoExport {
    pub name: lir::Atom,
    pub effects: u16,
    pub requires_caps: u16,
    pub stack_bound: u32,
}

/// Metadata collected for an import during extern word emission.
#[derive(Clone, Copy)]
pub(crate) struct ModInfoImport {
    pub name: lir::Atom,
}

pub struct ArmThumbBackend<'a> {
    pub module: &'a ModuleAst,
    pub src: &'a [u8],
    pub out: &'a mut dyn Output,
    pub mode: AsmMode,
    pub label_id: u32,
    pub str_len: usize,
    pub str_spans: [Span; 128],
    pub str_ids: [u32; 128],
    pub debug_trap_loc: bool,
    pub cur_word_id: u64,

    // --- S2 Phase 1: modinfo collection ---
    pub(crate) mi_exports: [ModInfoExport; 64],
    pub(crate) mi_export_count: usize,
    pub(crate) mi_imports: [ModInfoImport; 64],
    pub(crate) mi_import_count: usize,

    // --- S2 Phase 2: abi_hash ---
    pub expected_abi_hash: u64,

    // --- Slice 8: concurrency flag ---
    pub uses_tasks: bool,

    // --- Slice 5: scoped region allocation ---
    pub scoped_base: u32,
    pub scoped_slots: u32,
    pub scoped_next: u32,
}

impl<'a> ArmThumbBackend<'a> {
    fn module_has_interrupts(&self) -> bool {
        self.module.decls.iter().any(|d| {
            d.kind == DeclKind::Word
                && d.attrs
                    .iter()
                    .any(|a| matches!(a, AttrAst::Interrupt { .. }))
        })
    }

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
            str_len: 0,
            str_spans: [Span::UNKNOWN; 128],
            str_ids: [0u32; 128],
            debug_trap_loc,
            cur_word_id: 0,
            mi_exports: [ModInfoExport {
                name: lir::AT_EMPTY,
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
            }; 64],
            mi_export_count: 0,
            mi_imports: [ModInfoImport {
                name: lir::AT_EMPTY,
            }; 64],
            mi_import_count: 0,
            expected_abi_hash: 0,
            uses_tasks: false,
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

    pub(crate) fn emit_modinfo_section(&mut self) -> Result<(), CodegenError> {
        if self.mode != AsmMode::Object {
            return Ok(());
        }
        let export_count = self.mi_export_count;
        let import_count = self.mi_import_count;

        let mut export_entries: [lmod::modinfo::ExportEntry; 64] = [lmod::modinfo::ExportEntry {
            sym_hash: 0,
            name: b"",
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        }; 64];
        for i in 0..export_count {
            let mi = &self.mi_exports[i];
            let name = mi.name.as_bytes();
            let sym_hash = lmod::hash::fnv1a_u64(name);
            export_entries[i] = lmod::modinfo::ExportEntry {
                sym_hash,
                name,
                effects: mi.effects,
                requires_caps: mi.requires_caps,
                stack_bound: mi.stack_bound,
            };
        }

        let mut import_entries: [lmod::modinfo::ImportEntry; 64] = [lmod::modinfo::ImportEntry {
            sym_hash: 0,
            name: b"",
        }; 64];
        for i in 0..import_count {
            let name = self.mi_imports[i].name.as_bytes();
            let sym_hash = lmod::hash::fnv1a_u64(name);
            import_entries[i] = lmod::modinfo::ImportEntry { sym_hash, name };
        }

        let module_name = ophelpers::slice_span(self.src, self.module.name);

        let mut buf = [0u8; 8192];
        let abi_hash = if self.expected_abi_hash != 0 {
            self.expected_abi_hash
        } else {
            lmod::abi_hash::compute_abi_hash(2, 4, 32, lmod::modinfo::MODINFO_VER)
        };
        let size = match lmod::modinfo::encode_into(
            &mut buf,
            module_name,
            &export_entries[..export_count],
            &import_entries[..import_count],
            abi_hash,
            if self.module_has_interrupts() {
                lmod::modinfo::MODINFO_FLAG_HAS_ISR
            } else {
                0
            },
            &[],
        ) {
            Some(s) => s,
            None => return Err(CodegenError::ModInfoTooLarge),
        };

        self.out.write(b"\t.section .lang.modinfo\n");
        self.out.write(b"\t.byte ");
        if size > 0 {
            ophelpers::write_u32(self.out, buf[0] as u32);
            for i in 1..size {
                self.out.write(b",");
                ophelpers::write_u32(self.out, buf[i] as u32);
            }
        }
        self.out.write(b"\n");
        Ok(())
    }
}

impl<'a> CodegenBackend for ArmThumbBackend<'a> {
    fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        ArmThumbBackend::emit_prelude(self)
    }

    fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        ArmThumbBackend::emit_word(self, w)
    }

    fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        ArmThumbBackend::emit_postlude(self)
    }

    fn emit_extern_word(&mut self, name: &[u8]) -> Result<(), CodegenError> {
        ArmThumbBackend::emit_extern_word(self, name)
    }

    fn set_expected_abi_hash(&mut self, hash: u64) {
        self.expected_abi_hash = hash;
    }
}
