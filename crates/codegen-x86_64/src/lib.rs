#![no_std]
#![forbid(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
//! x86_64 code generation backend for Tyu.
//!
//! The public surface is centered on `X86_64HostedBackend`, with support
//! modules in `channel`, `mmio`, `ophelpers`, `postlude`, `prelude`,
//! `region`, `task`, `util`, and `word`.

extern crate alloc;

use codegen_core::{AsmMode, CodegenBackend, CodegenError, MmioWindowSpec};
use frontend::{
    parse::{AttrAst, DeclKind, ModuleAst, Output},
    span::Span,
};
use ir as lir;

pub mod channel;
pub mod mmio;
pub mod ophelpers;
pub mod postlude;
pub mod prelude;
pub mod region;
pub mod task;
pub(crate) mod util;
pub mod word;

// ---------------------------------------------------------------------------
// Modinfo collection (S2 Phase 1 — .lang.modinfo serialization)
// ---------------------------------------------------------------------------

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

/// Metadata for a single word in the `.lang.debug` section.
#[derive(Clone, Copy)]
pub(crate) struct DebugWordInfo {
    pub name: lir::Atom,
    pub net: i16,
    pub high: u32,
    pub effects: u16,
}

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
    pub uses_resources: bool,
    pub str_len: usize,
    pub str_spans: [Span; 128],
    pub str_ids: [u32; 128],
    pub debug_trap_loc: bool,
    pub cur_word_id: u64,
    /// Collected word info for `.lang.debug` section emission.
    /// Populated during `emit_word` when `debug_trap_loc` is set.
    pub(crate) debug_words: [Option<DebugWordInfo>; 4096],
    pub(crate) debug_word_count: usize,
    pub scoped_base: u32,
    pub scoped_slots: u32,
    pub scoped_next: u32,

    // --- P3: descriptor-sourced MMIO windows (D-7) ---
    /// Windows the board/runtime declares, copied from the langc driver
    /// (compiled platform descriptor, or the target's static defaults).
    /// Index `mmio_window_count` and beyond are `EMPTY`.
    pub mmio_windows: [MmioWindowSpec; 8],
    pub mmio_window_count: usize,

    // --- S2 Phase 1: modinfo collection ---
    pub(crate) mi_exports: [ModInfoExport; 64],
    pub(crate) mi_export_count: usize,
    pub(crate) mi_imports: [ModInfoImport; 64],
    pub(crate) mi_import_count: usize,

    // --- S2 Phase 2: abi_hash (abi-contract §5) ---
    /// Module-level ABI compatibility hash.  Set by the driver before
    /// `emit_postlude` is called (meaningless in Executable mode).
    pub expected_abi_hash: u64,
}

impl<'a> X86_64HostedBackend<'a> {
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
            uses_channels: false,
            uses_mmio: false,
            uses_regions: false,
            uses_tasks: false,
            uses_resources: false,
            str_len: 0,
            str_spans: [Span::UNKNOWN; 128],
            str_ids: [0u32; 128],
            debug_trap_loc,
            cur_word_id: 0,
            debug_words: [None; 4096],
            debug_word_count: 0,
            scoped_base: 0,
            scoped_slots: 0,
            scoped_next: 0,
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
            mmio_windows: [MmioWindowSpec::EMPTY; 8],
            mmio_window_count: 0,
        }
    }

    /// Set the MMIO windows this backend lowers against (P3, D-7). The langc
    /// driver supplies either the target's static defaults or the compiled
    /// platform descriptor's windows.
    pub fn set_mmio_windows(&mut self, windows: &[MmioWindowSpec]) -> Result<(), CodegenError> {
        if windows.len() > 8 {
            return Err(CodegenError::TooManyMmioWindows);
        }
        self.mmio_window_count = windows.len();
        self.mmio_windows = [MmioWindowSpec::EMPTY; 8];
        for (i, w) in windows.iter().enumerate() {
            self.mmio_windows[i] = *w;
        }
        Ok(())
    }

    /// The absolute address of a window-relative place (P4): `base + offset`,
    /// or `offset` alone when the window base is a link-time symbol (the
    /// emulated window — `__mmio_mem` is indexed by the offset directly).
    pub fn mmio_window_addr(&self, window: u16, offset: u32) -> Result<u64, CodegenError> {
        for i in 0..self.mmio_window_count {
            let w = &self.mmio_windows[i];
            if w.id == window {
                return Ok(w.base.unwrap_or(0).saturating_add(offset as u64));
            }
        }
        Err(CodegenError::NoMmioWindow)
    }

    pub fn fresh_label(&mut self) -> u32 {
        let id = self.label_id;
        self.label_id = self.label_id.wrapping_add(1);
        id
    }

    /// Emit the `.lang.modinfo` section (S2 Phase 1).
    /// Called from `emit_postlude` in object mode.
    pub(crate) fn emit_modinfo_section(&mut self) -> Result<(), CodegenError> {
        if self.mode != AsmMode::Object {
            return Ok(());
        }
        let export_count = self.mi_export_count;
        let import_count = self.mi_import_count;

        // Build export entries for the encoder.
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

        // Build import entries for the encoder.
        let mut import_entries: [lmod::modinfo::ImportEntry; 64] = [lmod::modinfo::ImportEntry {
            sym_hash: 0,
            name: b"",
        }; 64];
        for i in 0..import_count {
            let name = self.mi_imports[i].name.as_bytes();
            let sym_hash = lmod::hash::fnv1a_u64(name);
            import_entries[i] = lmod::modinfo::ImportEntry { sym_hash, name };
        }

        let module_name = util::slice_span(self.src, self.module.name);

        // Encode into a fixed-size stack buffer.
        let mut buf = [0u8; 8192];
        let abi_hash = if self.expected_abi_hash != 0 {
            self.expected_abi_hash
        } else {
            // Compute a default hash from the current target contract.
            lmod::abi_hash::compute_abi_hash(1, 8, 64, lmod::modinfo::MODINFO_VER)
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
            &[], // res_metas (no resources in current modules)
        ) {
            Some(s) => s,
            None => return Err(CodegenError::ModInfoTooLarge),
        };

        // Emit FASM section.
        self.out.write(b"section '.lang.modinfo'\n");
        self.out.write(b"  db ");
        if size > 0 {
            crate::ophelpers::write_u32(self.out, buf[0] as u32);
            for i in 1..size {
                self.out.write(b",");
                crate::ophelpers::write_u32(self.out, buf[i] as u32);
            }
        }
        self.out.write(b"\n");
        Ok(())
    }

    /// Emit the `.lang.debug` section (full-coverage word table) when
    /// `debug_trap_loc` is enabled.
    pub(crate) fn emit_debugsec_section(&mut self) -> Result<(), CodegenError> {
        if !self.debug_trap_loc {
            return Ok(());
        }
        let count = self.debug_word_count;
        if count == 0 || count > lmod::debugsec::MAX_DEBUG_ENTRIES {
            return Ok(());
        }

        // Build entries from the collected words.
        let mut entries: alloc::vec::Vec<lmod::debugsec::DebugEntry<'_>> =
            alloc::vec::Vec::with_capacity(count);
        for i in 0..count {
            if let Some(ref dw) = self.debug_words[i] {
                let name = dw.name.as_bytes();
                entries.push(lmod::debugsec::DebugEntry {
                    sym_hash: crate::util::fnv1a_u64(name),
                    name,
                    net: dw.net,
                    high: dw.high,
                    effects: dw.effects,
                });
            }
        }

        let mut buf = [0u8; 65536];
        let size = match lmod::debugsec::encode_into(&mut buf, &entries) {
            Some(s) => s,
            None => return Ok(()),
        };

        // Emit as FASM section with raw bytes.
        self.out.write(b"section '.lang.debug'\n  db ");
        if size > 0 {
            crate::ophelpers::write_u32(self.out, buf[0] as u32);
            for i in 1..size {
                self.out.write(b",");
                crate::ophelpers::write_u32(self.out, buf[i] as u32);
            }
        }
        self.out.write(b"\n");
        Ok(())
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
        X86_64HostedBackend::emit_extern_word(self, name)
    }

    fn set_expected_abi_hash(&mut self, hash: u64) {
        self.expected_abi_hash = hash;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink {
        buf: alloc::vec::Vec<u8>,
    }
    impl Output for Sink {
        fn write(&mut self, bytes: &[u8]) {
            self.buf.extend_from_slice(bytes);
        }
    }

    // Regression test for BUG-006: when the `.lang.modinfo` encoder cannot fit
    // the collected metadata into its fixed 8 KiB buffer, `emit_modinfo_section`
    // must report an error instead of silently dropping the section.
    #[test]
    fn modinfo_encoding_failure_is_an_error() {
        let mut src = alloc::vec::Vec::new();
        src.extend_from_slice(b"module ");
        src.extend(alloc::vec![b'x'; 4096]);
        src.extend_from_slice(b"; end;");
        let mod_ast = frontend::parse::Parser::new(&src).parse_module_ast().unwrap();

        let mut sink = Sink {
            buf: alloc::vec::Vec::new(),
        };
        let mut backend =
            X86_64HostedBackend::new(&mod_ast, &src, &mut sink, false, AsmMode::Object);
        let long_name = lir::Atom::new(&[b'y'; 32]).unwrap();
        backend.mi_exports = [ModInfoExport {
            name: long_name,
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        }; 64];
        backend.mi_export_count = 64;

        let err = backend.emit_modinfo_section().unwrap_err();
        assert_eq!(err, CodegenError::ModInfoTooLarge);
    }
}
