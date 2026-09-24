/// The assembly format the codegen backend should produce.
///
/// Distinct from [`EmitMode`]: `EmitMode` is the user-facing CLI choice;
/// `AsmMode` is the backend-internal output format selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsmMode {
    /// Self-contained ELF64 executable with entry point and inlined runtime.
    Executable,
    /// Relocatable ELF64 object file; runtime linked separately at assemble time.
    Object,
}

/// What the compiler should produce from the input module.
///
/// Only `Obj` is a production output. All other modes produce human-readable
/// text for inspection and debugging; they are not assemblable standalone.
///
/// The split between inspection and production is intentional and
/// architectural: `--emit=asm` no longer attempts to include the runtime,
/// eliminating the dual-scheduler divergence risk. The runtime lives in a
/// single canonical `.asm` file consumed only by `--emit=obj`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitMode {
    /// Dump the parsed AST as text. Inspection only.
    Ast,

    /// Dump the typed IR as text. Inspection only.
    Ir,

    /// Dump the stack-trace typecheck output as text. Inspection only.
    StackCheck,

    /// Emit target assembly as text. Inspection only.
    ///
    /// The output is human-readable and useful for debugging code generation,
    /// but it is NOT assemblable standalone — the runtime is not included.
    /// Use `--emit=obj` for any output that must be linked.
    Asm,

    /// Extract verification obligations and write `<Module>.obl.json`
    /// (static-verification.md slice P2). No codegen. A machine-readable,
    /// versioned artifact — the obligation interface (Q1), not an object.
    Obligations,

    /// Emit a relocatable object file. The sole production output path.
    ///
    /// The compiler runs the assembler on the generated text and produces an
    /// ELF (or target-appropriate) object that can be linked against the
    /// platform runtime.
    Obj,
}

impl EmitMode {
    /// All emit modes. Used for exhaustive iteration in tests.
    pub const ALL: [EmitMode; 6] = [
        EmitMode::Ast,
        EmitMode::Ir,
        EmitMode::StackCheck,
        EmitMode::Asm,
        EmitMode::Obligations,
        EmitMode::Obj,
    ];

    /// True if this mode produces a linkable/production artifact.
    pub fn is_production(self) -> bool {
        matches!(self, Self::Obj)
    }

    /// True if this mode produces human-readable text for inspection only.
    pub fn is_inspection(self) -> bool {
        !self.is_production()
    }
}
