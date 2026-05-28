use ir as lir;

/// Structured error type returned by all `CodegenBackend` methods.
///
/// Each variant carries enough context to produce a useful diagnostic without
/// requiring a string heap. The `code()` method returns a stable numeric code
/// for tooling compatibility and legacy diagnostic infrastructure.
///
/// # Migration note
/// The `Internal` variant is a shim for backend code that has not yet been
/// migrated to named variants. New backend code should use named variants
/// exclusively. The goal is to eliminate `Internal` over time.
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

    /// Migration shim: a legacy numeric code not yet given a named variant.
    ///
    /// Prefer adding a new named variant over using this in new code.
    Internal { code: u32 },
}

impl CodegenError {
    /// Stable numeric diagnostic code.
    ///
    /// Codes in the 8000–8999 range are owned by `codegen-core`.
    /// `Internal` passes through whatever code the legacy site provided.
    pub fn code(self) -> u32 {
        match self {
            Self::UnsupportedOp { .. }     => 8001,
            Self::MissingEntryPoint { .. } => 8002,
            Self::OutputCapacityExceeded   => 8003,
            Self::InvalidCast { .. }       => 8004,
            Self::UnsupportedEmitMode      => 8005,
            Self::MalformedStringLiteral   => 8006,
            Self::MalformedIr { .. }       => 8007,
            Self::Internal { code }        => code,
        }
    }
}
