use ir as lir;

/// Structured error type returned by all `CodegenBackend` methods.
///
/// Each variant carries enough context to produce a useful diagnostic without
/// requiring a string heap. The `code()` method returns a stable numeric code
/// for tooling compatibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodegenError {
    /// An IR opcode is not supported by this target backend.
    UnsupportedOp { op_name: &'static [u8] },

    /// The module does not contain a required entry-point word.
    MissingEntryPoint { name: &'static [u8] },

    /// The output buffer has been exhausted.
    OutputCapacityExceeded,

    /// A type cast between the given types is not valid on this target.
    InvalidCast { from: lir::TypeId, to: lir::TypeId },

    /// The requested emit mode is not supported for this target.
    UnsupportedEmitMode,

    /// A string literal in the source is malformed (bad escape sequence etc.).
    MalformedStringLiteral,

    /// An IR structural invariant was violated (e.g. duplicate block label).
    MalformedIr { detail: u32 },

    /// `AddrOf` without a compile-time constant address (runtime-only).
    UnsupportedAddrOf,

    /// A type property (bit width / signedness) could not be determined.
    UnknownTypeProperties { type_id: lir::TypeId },

    /// `CheckSubtype` IR op is not supported by the codegen backend.
    UnsupportedCheckSubtype,

    /// Exceeded maximum number of string literals per word.
    StringLiteralCapacityExceeded,

    /// Exceeded available scoped-allocation slots.
    ScopedAllocationOverflow,

    /// The `.lang.modinfo` section could not be encoded (fixed 64-entry
    /// export/import arrays or fixed-size buffer too small).
    ModInfoTooLarge,

    /// MMIO lowering requires a declared window of the needed kind (e.g. an
    /// emulated window on x86), but the backend has none (P3, D-7).
    NoMmioWindow,

    /// `set_mmio_windows` received more than the 8-window capacity.
    TooManyMmioWindows,
}

impl CodegenError {
    /// Stable numeric diagnostic code.
    ///
    /// Codes in the 8000–8999 range are owned by `codegen-core`.
    pub fn code(self) -> u32 {
        match self {
            Self::UnsupportedOp { .. } => 8001,
            Self::MissingEntryPoint { .. } => 8002,
            Self::OutputCapacityExceeded => 8003,
            Self::InvalidCast { .. } => 8004,
            Self::UnsupportedEmitMode => 8005,
            Self::MalformedStringLiteral => 8006,
            Self::MalformedIr { .. } => 8007,
            Self::UnsupportedAddrOf => 8008,
            Self::UnknownTypeProperties { .. } => 8009,
            Self::UnsupportedCheckSubtype => 8010,
            Self::StringLiteralCapacityExceeded => 8011,
            Self::ScopedAllocationOverflow => 8012,
            Self::ModInfoTooLarge => 8013,
            Self::NoMmioWindow => 8014,
            Self::TooManyMmioWindows => 8015,
        }
    }
}
