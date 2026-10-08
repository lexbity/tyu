//! Fuel (PLAN-WASM-1 §Q6/DEC-10): the ONLY preemption mechanism.
//!
//! `run` consumes a `u64` op budget deterministically — one unit per `OpKind`
//! step. Exhaustion is an observable outcome (`FuelExhausted`), never a
//! silent truncation, and is deterministic (no host clocks or RNGs anywhere
//! in the engine; the `Peripheral` boundary is the sole nondeterminism source,
//! P4). `fuel = 0` means "no budget" at the boundary level: the wasm layer
//! rejects it (E9151), host/CI callers may pass a concrete budget.

/// The default per-phrase op budget (PLAN-WASM-1 §Q6; recalibrated in P9 by
/// the bench gate — the doc number and this constant must never drift apart).
pub const DEFAULT_FUEL: u64 = 100_000_000;

/// The call-depth cap (NFR-4): deeper call chains are a stack-overflow trap
/// (code 10), matching the native `__stack_overflow` sink.
pub const CALL_DEPTH_CAP: usize = 256;

/// The data-stack slot cap (NFR-4): a full stack traps code 10.
pub const STACK_CAP: usize = 4096;

/// Check a caller-supplied budget. `0` is rejected by the wasm boundary
/// (E9151) but is legal here for host/CI tests that pass an explicit budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fuel(u64);

impl Fuel {
    pub const fn new(ops: u64) -> Fuel {
        Fuel(ops)
    }

    /// Consume one unit. `false` = exhausted (the step must not execute).
    #[inline]
    pub fn take(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }

    /// The remaining units (the trace header's `fuel` field, FR-35).
    pub fn remaining(self) -> u64 {
        self.0
    }
}
