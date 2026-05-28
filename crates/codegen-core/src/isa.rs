use frontend::span::Span;
use ir::{BlockId, CmpKind, Sig, TrapCode, TypeId};

/// Abstract instruction-set interface.
///
/// One `IsaEmitter` implementation exists per ISA (x86_64, arm-thumb2, rv32i).
/// Methods correspond directly to IR opcodes that are ISA-specific and carry
/// no platform or runtime dependency.
///
/// Platform-specific operations (output headers, extern declarations, trap
/// stubs, data sections) are handled by [`PlatformEmitter`].
/// Runtime intrinsics (task spawn/yield/join, channel send/recv, regions)
/// are handled by [`RuntimeEmitter`].
///
/// # Design rationale
/// Separating ISA from platform enables:
/// - Unit-testing instruction sequences with a `MockIsaEmitter` that captures
///   emitted ops as an enum vector, with no subprocess or assembler required.
/// - Sharing an ISA implementation across multiple platforms
///   (e.g. x86_64 Linux hosted and x86_64 Windows hosted).
/// - Adding a new platform without touching the ISA layer.
///
/// [`PlatformEmitter`]: crate::platform::PlatformEmitter
/// [`RuntimeEmitter`]: crate::runtime::RuntimeEmitter
pub trait IsaEmitter {
    type Error;

    // --- Frame setup (called once per word, before any block) ---------------

    /// Emit the word entry label and frame prologue.
    ///
    /// `local_slots` is the number of typed local variable slots.
    /// `scoped_slots` is the number of scoped-slice (region pointer) slots.
    fn emit_word_entry(
        &mut self,
        name: &[u8],
        local_slots: u32,
        scoped_slots: u32,
    ) -> Result<(), Self::Error>;

    /// Emit the word exit (post-`Ret` cleanup, if any).
    fn emit_word_exit(&mut self) -> Result<(), Self::Error>;

    /// Emit the label for a basic block.
    fn emit_block_label(&mut self, id: BlockId) -> Result<(), Self::Error>;

    /// Emit a stack-overflow guard at the entry of a word.
    fn emit_stack_guard(&mut self, overflow_label: &[u8]) -> Result<(), Self::Error>;

    // --- Stack primitives ---------------------------------------------------

    fn emit_const_i64(&mut self, v: i64, span: Span) -> Result<(), Self::Error>;
    fn emit_const_bool(&mut self, v: bool, span: Span) -> Result<(), Self::Error>;

    /// Duplicate the top-of-stack value of the given type.
    fn emit_dup(&mut self, ty: TypeId, span: Span) -> Result<(), Self::Error>;

    /// Discard the top-of-stack value of the given type.
    fn emit_drop(&mut self, ty: TypeId, span: Span) -> Result<(), Self::Error>;

    /// Swap the top two stack values.
    ///
    /// Stack effect: `( a:A  b:B -- b:B  a:A )`.
    /// `a` is the deeper value, `b` is the top.
    fn emit_swap(&mut self, a: TypeId, b: TypeId, span: Span) -> Result<(), Self::Error>;

    // --- Integer arithmetic -------------------------------------------------

    fn emit_add_i64(&mut self, span: Span) -> Result<(), Self::Error>;
    fn emit_sub_i64(&mut self, span: Span) -> Result<(), Self::Error>;
    fn emit_mul_i64(&mut self, span: Span) -> Result<(), Self::Error>;

    fn emit_cmp(
        &mut self,
        kind: CmpKind,
        out: TypeId,
        span: Span,
    ) -> Result<(), Self::Error>;

    // --- Boolean logic ------------------------------------------------------

    fn emit_and_bool(&mut self, span: Span) -> Result<(), Self::Error>;
    fn emit_or_bool(&mut self, span: Span) -> Result<(), Self::Error>;
    fn emit_not_bool(&mut self, span: Span) -> Result<(), Self::Error>;

    // --- Local variables ----------------------------------------------------

    fn emit_local_get(
        &mut self,
        slot: u16,
        ty: TypeId,
        span: Span,
    ) -> Result<(), Self::Error>;

    fn emit_local_set(
        &mut self,
        slot: u16,
        ty: TypeId,
        span: Span,
    ) -> Result<(), Self::Error>;

    // --- Type conversions ---------------------------------------------------

    fn emit_cast(
        &mut self,
        from: TypeId,
        to: TypeId,
        span: Span,
    ) -> Result<(), Self::Error>;

    fn emit_bitcast(
        &mut self,
        from: TypeId,
        to: TypeId,
        span: Span,
    ) -> Result<(), Self::Error>;

    // --- Memory access ------------------------------------------------------

    fn emit_load(&mut self, ty: TypeId, span: Span) -> Result<(), Self::Error>;
    fn emit_store(&mut self, ty: TypeId, span: Span) -> Result<(), Self::Error>;

    fn emit_ptr_add_const(
        &mut self,
        ty: TypeId,
        offset: u32,
        span: Span,
    ) -> Result<(), Self::Error>;

    fn emit_ptr_add_index(
        &mut self,
        ty: TypeId,
        scale: u32,
        span: Span,
    ) -> Result<(), Self::Error>;

    // --- Control flow -------------------------------------------------------

    fn emit_br(&mut self, target: BlockId, span: Span) -> Result<(), Self::Error>;

    fn emit_br_if(
        &mut self,
        then_tgt: BlockId,
        else_tgt: BlockId,
        span: Span,
    ) -> Result<(), Self::Error>;

    fn emit_ret(&mut self, span: Span) -> Result<(), Self::Error>;

    fn emit_call(
        &mut self,
        name: &[u8],
        sig: &Sig,
        may_suspend: bool,
        span: Span,
    ) -> Result<(), Self::Error>;

    // --- Runtime checks (contract / subtype) --------------------------------

    /// Emit a conditional trap: consume the `bool` on top of stack and
    /// trap with `code` if it is false.
    fn emit_trap_if_false(
        &mut self,
        code: TrapCode,
        span: Span,
    ) -> Result<(), Self::Error>;

    /// Emit a subtype range check: peek the top-of-stack value of `ty`,
    /// push a `bool` result. Stack effect: `( ty -- ty bool )`.
    fn emit_check_subtype(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Result<(), Self::Error>;
}
