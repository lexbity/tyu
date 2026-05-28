use frontend::span::Span;
use crate::error::CodegenError;

/// Platform-specific code generation interface.
///
/// One `PlatformEmitter` implementation exists per (ISA, OS/platform) pair,
/// e.g. `x86_64-unknown-linux-gnu`, `thumbv6m-none-eabi`.
///
/// Covers output-format headers, ABI declarations, section transitions,
/// trap stubs, and static data emission. ISA instruction encoding is handled
/// by [`IsaEmitter`]; runtime intrinsics by [`RuntimeEmitter`].
///
/// [`IsaEmitter`]: crate::isa::IsaEmitter
/// [`RuntimeEmitter`]: crate::runtime::RuntimeEmitter
pub trait PlatformEmitter {
    // --- Output format headers ----------------------------------------------

    /// Emit the file header for a self-contained executable, including the
    /// entry-point symbol declaration.
    fn emit_executable_header(
        &mut self,
        entry_label: &[u8],
    ) -> Result<(), CodegenError>;

    /// Emit the file header for a relocatable object file.
    fn emit_object_header(&mut self) -> Result<(), CodegenError>;

    // --- Extern / import declarations (--emit=obj) --------------------------

    /// Declare an external symbol that will be resolved at link time.
    fn emit_extern_decl(&mut self, sym: &[u8]) -> Result<(), CodegenError>;

    // --- Section transitions ------------------------------------------------

    /// Switch to the executable text section.
    fn emit_text_section(&mut self) -> Result<(), CodegenError>;

    /// Switch to the initialised data section.
    fn emit_data_section(&mut self) -> Result<(), CodegenError>;

    /// Switch to the zero-initialised BSS section.
    fn emit_bss_section(&mut self) -> Result<(), CodegenError>;

    // --- Trap / diagnostic stubs --------------------------------------------

    /// Emit an unconditional trap with the given numeric code.
    ///
    /// When `-g` is active the stub may additionally preserve source location
    /// information (e.g. by loading a span-encoded constant into a register
    /// before the trap instruction).
    fn emit_trap(&mut self, code: u32, span: Span) -> Result<(), CodegenError>;

    /// Emit the stack-overflow handler label and body.
    fn emit_stack_overflow_handler(&mut self) -> Result<(), CodegenError>;

    // --- Static data --------------------------------------------------------

    /// Emit a string literal into the data section.
    ///
    /// `id` is the unique integer assigned to this string by the backend's
    /// string intern table. The label is derived from `id` so the text
    /// section can reference it.
    fn emit_string_literal(
        &mut self,
        id: u32,
        bytes: &[u8],
    ) -> Result<(), CodegenError>;

    // --- Address-of static place --------------------------------------------

    /// Push the address of a named static place onto the value stack.
    ///
    /// If `const_addr` is `Some`, the place is at a known absolute address
    /// (MMIO register). Otherwise the address is resolved at link time.
    fn emit_addr_of(
        &mut self,
        place: &[u8],
        mutable: bool,
        const_addr: Option<u64>,
    ) -> Result<(), CodegenError>;

    // --- String constant push -----------------------------------------------

    /// Push the address and byte-length of an interned string literal.
    fn emit_const_str(
        &mut self,
        id: u32,
        byte_len: u32,
    ) -> Result<(), CodegenError>;
}
