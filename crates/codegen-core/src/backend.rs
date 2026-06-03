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
}
