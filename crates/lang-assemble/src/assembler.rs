use hosted::process;

pub enum AssembleError {
    /// The assembler binary could not be launched (not found or not executable).
    LaunchFailed,
    /// The assembler exited with a non-zero status code.
    NonZeroExit,
}

/// Common interface over concrete assembler backends.
///
/// Each implementor wraps one assembler tool (FASM, GAS, …) and knows how to
/// invoke it. Callers obtain a concrete implementor through
/// [`crate::driver::make_driver`] and dispatch uniformly via this trait.
pub trait AssemblerDriver {
    /// Assemble `input` (a text assembly file) into `output` (object file).
    fn assemble(&self, input: &[u8], output: &[u8]) -> Result<(), AssembleError>;
}

// ---------------------------------------------------------------------------
// FASM backend
// ---------------------------------------------------------------------------

/// Flat Assembler (FASM) backend.
///
/// Invokes: `<path> <input> <output>`
pub struct FasmDriver<'a> {
    pub path: &'a [u8],
}

impl<'a> AssemblerDriver for FasmDriver<'a> {
    fn assemble(&self, input: &[u8], output: &[u8]) -> Result<(), AssembleError> {
        let status = process::run(self.path, &[input, output])
            .map_err(|_| AssembleError::LaunchFailed)?;
        if status.code != 0 {
            return Err(AssembleError::NonZeroExit);
        }
        Ok(())
    }
}
