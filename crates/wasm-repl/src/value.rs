//! Concrete stack values (PLAN-WASM-1 §5, §7.1).
//!
//! The interpreter models the native data stack exactly as the backends do:
//! single-slot values, two's-complement wrapping i64 arithmetic, booleans as
//! 0/1 slots, and pointers as raw addresses (§Q11 — the native ABI is
//! reference-shaped: aggregates live at addresses; `dup` copies the reference,
//! never the aggregate, and the aliasing probes pin that).
//!
//! [`Value`] keeps a *kind* tag ([`Slot`]) so the L2/L3 enforcement layers
//! (§Q4) can validate what they must (typed pops, `Cast`/subtype ranges). The
//! engine NEVER branches on a value's representation: `Bool(false)` is
//! observably `0` and arithmetic on it would be a type error caught at
//! compile time (the module is `ir::verify_word`'d before it runs).

use ir::TypeId;

/// A single data-stack slot value. `Bool` is a distinct tag, not a bit trick:
/// type-correct programs never mix kinds, and the L2 checks make a mismatch a
/// trap instead of silent garbage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    /// An address (ptr / ptr_mut / region / slice / mmio aperture cover).
    Ptr(u64),
}

impl Value {
    /// Truthiness as the native runtime observes it: `Cmp`/`AndBool`/…
    /// produce 0/1 slots and `TrapIfFalse`/`BrIf` test non-zero.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Int(v) => *v != 0,
            Value::Bool(b) => *b,
            Value::Ptr(p) => *p != 0,
        }
    }

    /// The value as a raw signed slot (hex-facing and `testio.write-byte`
    /// byte extraction). Booleans are 0/1; pointers keep their address bits.
    pub fn as_i64(&self) -> i64 {
        match self {
            Value::Int(v) => *v,
            Value::Bool(b) => *b as i64,
            Value::Ptr(p) => *p as i64,
        }
    }

    pub fn as_u64(&self) -> u64 {
        self.as_i64() as u64
    }
}

/// A stack slot: a value plus the IR `TypeId` it entered the stack with, so
/// the L2/L3 enforcement layers can type-check typed pops and subtype ranges
/// without re-deriving anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Slot {
    pub ty: TypeId,
    pub val: Value,
}

impl Slot {
    pub const fn int(ty: TypeId, v: i64) -> Slot {
        Slot {
            ty,
            val: Value::Int(v),
        }
    }

    pub const fn bool(v: bool) -> Slot {
        Slot {
            ty: ir::TY_BOOL,
            val: Value::Bool(v),
        }
    }

    pub const fn ptr(ty: TypeId, p: u64) -> Slot {
        Slot {
            ty,
            val: Value::Ptr(p),
        }
    }
}
