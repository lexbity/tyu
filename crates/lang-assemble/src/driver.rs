use crate::assembler::{AssembleError, AssemblerDriver, FasmDriver};
use crate::config::Config;
use codegen_core::AssemblerKind;
use hosted::diag;

/// Default binary name for each assembler kind.
fn default_assembler_path(kind: AssemblerKind) -> &'static [u8] {
    match kind {
        AssemblerKind::Fasm => b"fasm",
        AssemblerKind::GasArm => b"arm-none-eabi-as",
        AssemblerKind::GasRiscV => b"riscv32-elf-as",
        // D-W14: the wasm32 target emits its final binary directly — there
        // is no assembler, and this driver's text-assembly flow never meets
        // it (a wasm build resolving an assembler is a caller bug).
        AssemblerKind::None => b"",
    }
}

pub fn run(config: &Config) -> i32 {
    let spec = config.target.spec();
    let path = config
        .assembler_path
        .unwrap_or_else(|| default_assembler_path(spec.assembler));

    match spec.assembler {
        AssemblerKind::Fasm => run_with_driver(config, &FasmDriver { path }),
        // D-W14: None (wasm32) has no text assembly to assemble — its
        // backend emits the final binary. This driver is a text-assembly
        // tool; reaching it with None is a caller bug, surfaced loudly.
        AssemblerKind::None | AssemblerKind::GasArm | AssemblerKind::GasRiscV => {
            let _ = diag::error_simple(
                2005,
                b"assembler backend not yet implemented for this target",
            );
            2
        }
    }
}

fn run_with_driver(config: &Config, driver: &dyn AssemblerDriver) -> i32 {
    match driver.assemble(config.input, config.out_path) {
        Ok(()) => 0,
        Err(AssembleError::LaunchFailed) => {
            let _ = diag::error_simple(2002, b"failed to run assembler");
            2
        }
        Err(AssembleError::NonZeroExit) => {
            let _ = diag::error_simple(2003, b"assembler failed");
            2
        }
    }
}
