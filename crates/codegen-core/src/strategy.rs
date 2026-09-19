//! MMIO lowering classification matrix (design doc §5.6, decision D-3/D-9).
//!
//! Each backend classifies every `(strategy, aperture-kind, op)` combination as
//! either `Supported(pattern-name)` or `Unsupported(reason)` via an exhaustive
//! `fn mmio_cell`. `codegen-core` renders the three tables and diffs them
//! against a checked-in golden (`strategy_matrix.rs`) — a cell changing
//! without its golden changing is impossible. The `E3642`/`E3643` rows are
//! compile-time checks (R1/R2) enforced by the typechecker and IR verifier,
//! not backend lowerings; the golden names them as such.

use alloc::string::String;
use alloc::string::ToString;
use core::fmt::Write;

use ir::{ApertureKind, WriteKind};

/// The MMIO op being classified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmioOp {
    Load,
    Store,
    LoadField,
    StoreField,
}

impl MmioOp {
    pub fn as_str(self) -> &'static str {
        match self {
            MmioOp::Load => "ld",
            MmioOp::Store => "st",
            MmioOp::LoadField => "field-ld",
            MmioOp::StoreField => "field-st",
        }
    }
}

/// The classification of a `(strategy, aperture-kind, op)` cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrategyCell {
    /// The backend lowers this combination; the name is the asm pattern.
    Supported { pattern: &'static str },
    /// The backend rejects this combination; the reason names the cell.
    Unsupported { reason: &'static str },
}

/// Render one backend's full matrix table.
pub fn render_strategy_matrix(
    backend: &str,
    cell: impl Fn(WriteKind, ApertureKind, MmioOp) -> StrategyCell,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {backend} MMIO strategy matrix (design doc §5.6)");
    for strategy in [WriteKind::Plain, WriteKind::W1s, WriteKind::W1c, WriteKind::Xor] {
        for kind in [ApertureKind::Bus, ApertureKind::Emulated] {
            for op in [MmioOp::Load, MmioOp::Store, MmioOp::LoadField, MmioOp::StoreField] {
                let cell = cell(strategy, kind, op);
                let rendered = match cell {
                    StrategyCell::Supported { pattern } => pattern.to_string(),
                    StrategyCell::Unsupported { reason } => reason.to_string(),
                };
                let _ = writeln!(
                    out,
                    "{:<6} {:<9} {:<9} {}",
                    strategy_str(strategy),
                    kind_str(kind),
                    op.as_str(),
                    rendered
                );
            }
        }
    }
    out.push_str(GUARD_ROWS);
    out.push_str(FIXTURE_MAP);
    out
}

/// The compile-time guard rows (R1/R2), rendered after every backend table.
pub const GUARD_ROWS: &str = "# compile-time guards (all backends, any strategy / aperture kind)\n\
any any field-st-on-effectful E3642 phantom-read (R1)\n\
any any access-over-atomic_max E3643 over-wide (R2)\n\
";

/// Per-cell fixture coverage map (design doc §5.6 / P5): every `Supported`
/// row is exercised by a QEMU fixture. `RMW` cells cover `field-ld` too.
pub const FIXTURE_MAP: &str = "\n\
# fixture coverage (crates/execution-tests/fixtures)\n\
#   plain  bus       ld/st      mmio_smoke_{arm,riscv}\n\
#   w1s    bus       st         mmio_strategies_{arm,riscv} (RMW or/orr)\n\
#   w1c    bus       st         mmio_strategies_{arm,riscv} (RMW and/bic)\n\
#   xor    bus       st         mmio_strategies_{arm,riscv} (RMW xor/eor)\n\
#   field-ld/st       bus       mmio_strategies_* (FIFO32 field via brace block)\n\
#   plain  emulated  ld/st      mmio_smoke_x86\n\
#   w1s/w1c/xor emulated st      mmio_strategies_x86 (RMW or / RMW andn / RMW xor)\n\
#   E3642  any        field-st-on-effectful  platform_resolution::e3642_*\n\
#   E3643  any        over-atomic_max        platform_resolution::e3643_over_wide\n\
";

fn strategy_str(s: WriteKind) -> &'static str {
    match s {
        WriteKind::Plain => "plain",
        WriteKind::W1s => "w1s",
        WriteKind::W1c => "w1c",
        WriteKind::Xor => "xor",
    }
}

fn kind_str(k: ApertureKind) -> &'static str {
    match k {
        ApertureKind::Bus => "bus",
        ApertureKind::Emulated => "emulated",
    }
}

/// The compile-time guards the matrix's `E3642`/`E3643` cells name (R1/R2).
pub const PHANTOM_READ: &str = "E3642 phantom-read (R1)";
pub const OVER_WIDE: &str = "E3643 over-wide (R2)";