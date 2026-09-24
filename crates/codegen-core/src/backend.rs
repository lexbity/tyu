use crate::error::CodegenError;
use ir as lir;

/// The code generation interface implemented by each target backend.
///
/// The driver calls these three methods in order for every module compiled:
///
/// 1. `emit_prelude` — output-format header, entry point, extern declarations.
/// 2. `emit_word` — called once per IR word in module order.
/// 3. `emit_postlude` — data sections, runtime data structures, string literals.
///
/// Backends are selected by the driver based on the `Target` and `EmitMode`.
/// Each backend is responsible for a specific (ISA, platform) pair.
///
/// # Error handling
/// Methods return `CodegenError` rather than a raw `u32` so that the driver
/// can produce structured diagnostics. The `CodegenError::code()` method
/// provides a stable numeric code for backwards compatibility.
pub trait CodegenBackend {
    fn emit_prelude(&mut self) -> Result<(), CodegenError>;
    fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError>;
    fn emit_postlude(&mut self) -> Result<(), CodegenError>;

    /// Emit an `extrn` (or equivalent) declaration for an imported word
    /// symbol so the assembler can resolve cross-module calls at link time.
    ///
    /// Called once per imported word, after `emit_prelude` and before any
    /// `emit_word` calls. The default is a no-op (useful for inspection modes
    /// and stub backends that do not produce real object files).
    fn emit_extern_word(&mut self, _name: &[u8]) -> Result<(), CodegenError> {
        Ok(())
    }

    /// Set the ABI compatibility hash that the backend will embed in the
    /// output's `.lang.modinfo` section during `emit_postlude`.
    /// This is called by the driver between word emission and postlude.
    fn set_expected_abi_hash(&mut self, _hash: u64) {}

    /// Set the module's `platform_hash` (P6, decision D-5): the canonical
    /// hash of the compiled platform descriptor the module was built against,
    /// or 0 for an unplatformed (MMIO-free) module. Stamped into the
    /// `.lang.modinfo` v4 header; the loader enforces it (E5220).
    fn set_platform_hash(&mut self, _hash: u64) {}

    /// Slice P4 (static-verification.md §6.2/§7.2): when `elide` is true the
    /// backend is told that every emulated-aperture MMIO bounds obligation of
    /// the word being emitted carries a discharged verdict, so the C7 bounds
    /// check can be skipped. Per-word granularity (Q8) — a word with any open
    /// access must retain all checks; the lowering only arms this from
    /// `Undischarged` mode, so `--checks=all` output is untouched (FR-5).
    /// Default no-op: backends without emulated-aperture checks ignore it.
    fn set_mmio_checks_discharged(&mut self, _elide: bool) {}
}

/// Merge one aperture-use entry into a backend's module aperture table (P6 §5.5):
/// first use wins for the aperture facts, later uses OR in the access mask.
/// Shared by every backend so the modinfo aperture table is derived identically
/// (NFR-6: one implementation).
pub fn merge_aperture_use(table: &mut [ir::ApertureUse; 8], count: &mut usize, wu: &ir::ApertureUse) {
    for existing in table.iter_mut().take(*count) {
        if existing.id == wu.id {
            existing.access_mask |= wu.access_mask;
            return;
        }
    }
    if *count < 8 {
        table[*count] = *wu;
        *count += 1;
    }
}
