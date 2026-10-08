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

use alloc::vec;
use alloc::vec::Vec;
use ir::TypeId;

/// The kind tag of a [`Value`] (L2a's checked-pop vocabulary, DEC-23).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValKind {
    Int,
    Bool,
    Ptr,
}

impl ValKind {
    /// The kind's display name (the `declared` field of a contract-trap
    /// `TrapDetail` — §5.3 of PLAN-WASM-2).
    pub fn name(self) -> &'static [u8] {
        match self {
            ValKind::Int => b"i64",
            ValKind::Bool => b"bool",
            ValKind::Ptr => b"ptr",
        }
    }
}

/// The canonical `0x%016x` render of a raw slot (the `value_hex` field of a
/// UI/`StackCell`). One formatter for every surface (session, engine detail,
/// debug projection) so hex bytes never drift between renderers (I-11).
pub fn format_hex(v: u64) -> Vec<u8> {
    let mut out = vec![b'0', b'x'];
    let mut digits = [0u8; 16];
    for (i, d) in digits.iter_mut().enumerate() {
        let nib = ((v >> (4 * (15 - i))) & 0xF) as u8;
        *d = if nib < 10 {
            b'0' + nib
        } else {
            b'a' + (nib - 10)
        };
    }
    out.extend_from_slice(&digits);
    out
}

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
    /// The value's kind tag (L2a, DEC-23): every engine pop names the kind it
    /// expects; a mismatch is trap 20 with the op's span (never a coercion).
    /// The raw-slot accessors below remain for *display* and the `testio.*`
    /// byte surface only — the engine's execution paths never call them.
    pub fn kind(&self) -> ValKind {
        match self {
            Value::Int(_) => ValKind::Int,
            Value::Bool(_) => ValKind::Bool,
            Value::Ptr(_) => ValKind::Ptr,
        }
    }

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

    /// The value's canonical hex render (the `value_hex` UI field). Display
    /// only — the one sanctioned `as_u64` consumer so the engine's grep gate
    /// ("no raw-slot reads outside display") stays a total check.
    pub fn hex(&self) -> Vec<u8> {
        format_hex(self.as_u64())
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
