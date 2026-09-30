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
        let status =
            process::run(self.path, &[input, output]).map_err(|_| AssembleError::LaunchFailed)?;
        if status.code != 0 {
            return Err(AssembleError::NonZeroExit);
        }
        mark_executable_if_elf_exec(output);
        Ok(())
    }
}

/// Make the assembled output executable iff it is an ELF *executable*
/// (`e_type == ET_EXEC`). fasm sets the exec bit itself on some versions
/// (1.73.35) and not others (1.73.34, the ubuntu-26.04 package) — the
/// hosted pipeline's output must be runnable regardless of the assembler
/// build. Object files (`e_type == ET_REL`) are left untouched.
fn mark_executable_if_elf_exec(path: &[u8]) {
    let Ok(blob) = hosted::fs::read_file(path) else {
        return;
    };
    if blob.as_slice().len() < 18 || !blob.as_slice().starts_with(b"\x7fELF") {
        return;
    }
    let e_type = u16::from_le_bytes([blob.as_slice()[16], blob.as_slice()[17]]);
    if e_type != 2 {
        return; // ET_REL / ET_DYN — not a runnable executable
    }
    let _ = hosted::fs::mark_executable(path);
}
