use crate::error::CodegenError;
use ir::TypeId;

/// Channel payload classification.
///
/// Determines how a value is marshalled through the 8-byte channel slot.
/// Every channel entry is exactly 8 bytes; values that do not fit are
/// heap-boxed and transmitted by pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelPayloadKind {
    /// A primitive scalar that fits in a register (i8–i64, bool, ptr).
    ///
    /// `bits` is the natural width; `signed` controls sign-extension when
    /// the value is widened to fill the 8-byte slot; `is_bool` triggers
    /// canonicalisation to 0/1.
    Primitive {
        bits: u16,
        signed: bool,
        is_bool: bool,
    },

    /// A composite value too large for a register slot — heap-boxed and
    /// transmitted by pointer to the heap allocation.
    BoxCopy { bytes: u32 },

    /// Any other word-sized value (pointer-width) requiring no special
    /// marshalling.
    Word,
}

/// Runtime intrinsic emission interface.
///
/// One `RuntimeEmitter` implementation exists per platform runtime
/// (e.g. the cooperative scheduler on `x86_64-unknown-linux-gnu`,
/// a bare-metal RTOS shim on `thumbv6m-none-eabi`).
///
/// Covers cooperative scheduler primitives (task spawn/yield/join/sleep),
/// channel send/receive, region entry, and MMIO place references.
/// ISA instruction encoding is handled by [`IsaEmitter`].
/// Platform headers and data sections are handled by [`PlatformEmitter`].
///
/// # Single source of truth
/// Implementations must NOT re-emit scheduler logic inline. The runtime
/// is defined in a single canonical assembly file per target
/// (`runtime/<arch>/<platform>/runtime.asm`). `--emit=obj` links against
/// a pre-assembled object derived from that file. `--emit=asm` (inspection
/// mode) does not include the runtime at all.
///
/// [`IsaEmitter`]: crate::isa::IsaEmitter
/// [`PlatformEmitter`]: crate::platform::PlatformEmitter
pub trait RuntimeEmitter {
    // --- Task primitives ----------------------------------------------------

    /// Emit a task spawn: allocate a task slot, copy the task body, push
    /// the task handle.
    fn emit_task_spawn(&mut self, name: &[u8], task_ty: TypeId) -> Result<(), CodegenError>;

    /// Emit a cooperative yield point.
    fn emit_task_yield(&mut self) -> Result<(), CodegenError>;

    /// Emit a blocking join on the task handle at the top of the stack.
    fn emit_task_join(&mut self) -> Result<(), CodegenError>;

    /// Emit a millisecond sleep.
    fn emit_task_sleep_ms(&mut self) -> Result<(), CodegenError>;

    /// Emit a microsecond sleep.
    fn emit_task_sleep_us(&mut self) -> Result<(), CodegenError>;

    // --- Channel primitives -------------------------------------------------

    /// Emit channel creation for the given element type.
    fn emit_channel_make(&mut self, ty: TypeId) -> Result<(), CodegenError>;

    /// Emit a channel send.
    ///
    /// `ty` is the element type; `kind` drives payload marshalling.
    fn emit_channel_send(
        &mut self,
        ty: TypeId,
        kind: ChannelPayloadKind,
    ) -> Result<(), CodegenError>;

    /// Emit a channel receive.
    ///
    /// `ty` is the element type; `kind` drives payload unmarshalling.
    fn emit_channel_recv(
        &mut self,
        ty: TypeId,
        kind: ChannelPayloadKind,
    ) -> Result<(), CodegenError>;

    // --- Region / scoped allocation -----------------------------------------

    /// Emit region entry: push a scoped slice of `len` elements of `ty`.
    fn emit_region_enter(&mut self, ty: TypeId, len: u32) -> Result<(), CodegenError>;

    // --- MMIO ---------------------------------------------------------------

    /// Push the address of an MMIO-mapped register bank.
    fn emit_mmio_place(&mut self, place: &[u8], addr: u64) -> Result<(), CodegenError>;
}
