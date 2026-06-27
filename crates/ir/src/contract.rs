//! Effect / Capability / Context model types.
//!
//! These are the pure-data types for the effect-context model
//! (effect-context-model.md) and the static stack-bound analysis
//! (stack-bound-analysis.md).  They carry no behaviour beyond their
//! algebraic laws; the checking logic lives in the semantics crate.
//!
//! Wire encodings follow abi-contract.md §4.

/// A set of effects a word may exhibit during execution.
///
/// | Bit | Constant    | Meaning                       |
/// |-----|-------------|-------------------------------|
/// | 0   | `SUSPEND`   | may yield to scheduler        |
/// | 1   | `INTERRUPT` | runs with ISR semantics       |
/// | 2   | `DIVERGE`   | may not return (termination)  |
/// | 3   | `MMIO`      | performs volatile MMIO access |
/// | 4   | `ALLOC`     | may allocate from a region    |
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectSet(u16);

impl EffectSet {
    pub const SUSPEND: u16 = 1 << 0;
    pub const INTERRUPT: u16 = 1 << 1;
    pub const DIVERGE: u16 = 1 << 2;
    pub const MMIO: u16 = 1 << 3;
    pub const ALLOC: u16 = 1 << 4;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn minus(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn without(self, bit: u16) -> Self {
        Self(self.0 & !bit)
    }
}

// ---------------------------------------------------------------------------
// 2.2 CapSet — bits 0..4 reserved; bits 5..15 free for future capabilities.
// ---------------------------------------------------------------------------

/// A set of capabilities a word requires (or a context grants).
///
/// | Bit | Constant         | Meaning                        |
/// |-----|------------------|--------------------------------|
/// | 0   | `WRITE`          | write/borrow-mut to a resource |
/// | 1   | `SUSPENDABLE`    | suspension permitted here      |
/// | 2   | `COPYABLE`       | `dup` is permitted on a type   |
/// | 3   | `BORROW_LIVE`    | a non-escaping borrow is live  |
/// | 4   | `BOUNDED_STACK`  | bounded-stack(N) budget active |
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct CapSet(u16);

impl CapSet {
    pub const WRITE: u16 = 1 << 0;
    pub const SUSPENDABLE: u16 = 1 << 1;
    pub const COPYABLE: u16 = 1 << 2;
    pub const BORROW_LIVE: u16 = 1 << 3;
    pub const BOUNDED_STACK: u16 = 1 << 4;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    /// C1 check: is every bit in `self` also set in `grants`?
    pub const fn subset_of(self, grants: Self) -> bool {
        (self.0 & !grants.0) == 0
    }

    pub const fn contains(self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

// ---------------------------------------------------------------------------
// 2.3 High — the data-stack high-water lattice.
// ---------------------------------------------------------------------------

/// Data-stack high-water mark relative to entry.
///
/// `Slots(n)` — a known finite bound of `n` stack slots.
/// `Top` — no finite bound (unbounded non-tail recursion).
///
/// Invariant: `Slots(n)` with `n` non-negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum High {
    Slots(u32),
    Top,
}

impl High {
    /// The minimal possible bound.
    pub const ZERO: Self = High::Slots(0);

    pub const fn is_top(self) -> bool {
        matches!(self, High::Top)
    }

    pub const fn is_finite(self) -> bool {
        matches!(self, High::Slots(_))
    }

    pub fn unwrap_slots(self) -> u32 {
        assert!(!self.is_top(), "unwrap_slots on Top");
        if let High::Slots(n) = self {
            n
        } else {
            0
        }
    }

    /// Add a signed offset.  Saturates at 0 on the low side; overflows
    /// to `Top` on the high side.
    pub fn add_offset(self, offset: i16) -> Self {
        match self {
            High::Top => High::Top,
            High::Slots(n) => {
                if offset < 0 {
                    let abs = (-offset) as u32;
                    if abs >= n {
                        High::Slots(0)
                    } else {
                        High::Slots(n - abs)
                    }
                } else {
                    match n.checked_add(offset as u32) {
                        Some(v) => High::Slots(v),
                        None => High::Top,
                    }
                }
            }
        }
    }

    /// Does `self` exceed `ceiling`?  `Top` exceeds any finite ceiling;
    /// nothing exceeds a `Top` ceiling (including `Top` itself).
    pub fn exceeds(self, ceiling: Self) -> bool {
        match (self, ceiling) {
            (High::Top, High::Top) => false,
            (High::Top, _) => true,
            (_, High::Top) => false,
            (High::Slots(a), High::Slots(b)) => a > b,
        }
    }

    fn max(self, other: Self) -> Self {
        match (self, other) {
            (High::Top, _) | (_, High::Top) => High::Top,
            (High::Slots(a), High::Slots(b)) => High::Slots(a.max(b)),
        }
    }
}

// ---------------------------------------------------------------------------
// 2.4 StackBound — the composable (net, high) pair.
// ---------------------------------------------------------------------------

/// The composable stack-bound pair.
///
/// - `net`: change in stack depth after execution (may be negative).
/// - `high`: peak depth reached during execution, relative to entry.
///
/// Invariants: `high ≥ 0` and `high ≥ net` reinterpreted through `High`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackBound {
    pub net: i16,
    pub high: High,
}

impl StackBound {
    /// Identity element: `(0, 0)`.
    pub const ID: Self = StackBound {
        net: 0,
        high: High::Slots(0),
    };

    /// Sequence composition `w1 w2`.
    ///
    /// ```text
    /// net  = net1 + net2
    /// high = max(high1, net1 + high2)
    /// ```
    pub fn compose(self, next: Self) -> Self {
        let net = self.net.wrapping_add(next.net);
        let shifted = shift_high(self.net, next.high);
        let high = High::max(self.high, shifted);
        StackBound { net, high }
    }

    /// Branch merge (`if`/`else`).  Caller must ensure `net == other.net`.
    ///
    /// ```text
    /// net  = net1  (== net2)
    /// high = max(high1, high2)
    /// ```
    pub fn branch_max(self, other: Self) -> Self {
        debug_assert!(
            self.net == other.net,
            "branch_max requires equal net: {} != {}",
            self.net,
            other.net,
        );
        let high = High::max(self.high, other.high);
        StackBound {
            net: self.net,
            high,
        }
    }

    /// Wire encoding for the `high` field.  `Top ⇒ 0xFFFF_FFFF`.
    pub fn wire_u32(self) -> u32 {
        match self.high {
            High::Slots(n) => n,
            High::Top => 0xFFFF_FFFF,
        }
    }
}

/// Shift a `High` by a signed `net` offset.
///
/// Equivalent to `net + high` in the lattice, saturating at 0 below and
/// `Top` above.
fn shift_high(net: i16, high: High) -> High {
    match high {
        High::Top => High::Top,
        High::Slots(h) => {
            if net < 0 {
                let abs = (-net) as u32;
                if abs >= h {
                    High::Slots(0)
                } else {
                    High::Slots(h - abs)
                }
            } else {
                match h.checked_add(net as u32) {
                    Some(v) => High::Slots(v),
                    None => High::Top,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2.5 Context — the ambient environment.
// ---------------------------------------------------------------------------

/// The ambient context threaded through the stack checker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub grants: CapSet,
    pub forbids: EffectSet,
    pub ceiling: High,
}

impl Context {
    pub const fn new(grants: CapSet, forbids: EffectSet, ceiling: High) -> Self {
        Self {
            grants,
            forbids,
            ceiling,
        }
    }

    /// Default ambient context: no grants, no forbids, unbounded ceiling.
    pub const fn default() -> Self {
        Self {
            grants: CapSet::empty(),
            forbids: EffectSet::empty(),
            ceiling: High::Top,
        }
    }
}

// ---------------------------------------------------------------------------
// Core formatting helpers (debug-printable, no_std-friendly).
// ---------------------------------------------------------------------------

use core::fmt;

impl fmt::Debug for EffectSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut write_bit = |name: &str, bit: u16| -> fmt::Result {
            if self.contains(bit) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str(name)?;
            }
            Ok(())
        };
        write_bit("SUSPEND", EffectSet::SUSPEND)?;
        write_bit("INTERRUPT", EffectSet::INTERRUPT)?;
        write_bit("DIVERGE", EffectSet::DIVERGE)?;
        write_bit("MMIO", EffectSet::MMIO)?;
        write_bit("ALLOC", EffectSet::ALLOC)?;
        if first {
            f.write_str("∅")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CapSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut write_bit = |name: &str, bit: u16| -> fmt::Result {
            if self.contains(bit) {
                if !first {
                    f.write_str(" | ")?;
                }
                first = false;
                f.write_str(name)?;
            }
            Ok(())
        };
        write_bit("WRITE", CapSet::WRITE)?;
        write_bit("SUSPENDABLE", CapSet::SUSPENDABLE)?;
        write_bit("COPYABLE", CapSet::COPYABLE)?;
        write_bit("BORROW_LIVE", CapSet::BORROW_LIVE)?;
        write_bit("BOUNDED_STACK", CapSet::BOUNDED_STACK)?;
        if first {
            f.write_str("∅")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// §5 — abi_hash
// ---------------------------------------------------------------------------

/// ABI contract version — bump when the hash input changes meaningfully.
pub const ABI_CONTRACT_VERSION: u64 = 1;

/// Compute a 64-bit hash of a word's ABI contract for cross-module
/// compatibility checking.  Two modules that import/export the same word
/// must produce the same hash; a mismatch → separate compilation error.
///
/// Inputs folded into the hash:
///   - contract version (u64)
///   - in_len (u8)
///   - out_len (u8)
///   - input type atoms (up to 8 × 32 bytes)
///   - output type atoms (up to 8 × 32 bytes)
///   - performs (EffectSet bits)
///   - requires (CapSet bits)
///   - bound (net i16 + high u32)
///   - slot_bytes (bytes per data-stack slot, target-specific)
pub fn abi_hash(
    sig_in: &[u8],
    sig_out: &[u8],
    performs: EffectSet,
    requires: CapSet,
    bound: StackBound,
    slot_bytes: u32,
) -> u64 {
    let mut h: u64 = ABI_CONTRACT_VERSION;
    h = h.wrapping_mul(31).wrapping_add(slot_bytes as u64);
    // Signature inputs
    for &b in sig_in {
        h = h.wrapping_mul(31).wrapping_add(b as u64);
    }
    h = h.wrapping_mul(31).wrapping_add(0xff); // separator
                                               // Signature outputs
    for &b in sig_out {
        h = h.wrapping_mul(31).wrapping_add(b as u64);
    }
    h = h.wrapping_mul(31).wrapping_add(0xff); // separator
                                               // Effects and capabilities
    h = h.wrapping_mul(31).wrapping_add(performs.bits() as u64);
    h = h.wrapping_mul(31).wrapping_add(requires.bits() as u64);
    // Stack bound
    h = h.wrapping_mul(31).wrapping_add(bound.net as u64);
    h = h.wrapping_mul(31).wrapping_add(bound.wire_u32() as u64);
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- EffectSet --------------------------------------------------------

    #[test]
    fn effect_set_empty() {
        let e = EffectSet::empty();
        assert!(e.is_empty());
        assert_eq!(e.bits(), 0);
    }

    #[test]
    fn effect_set_singleton() {
        let s = EffectSet::from_bits(EffectSet::SUSPEND);
        assert!(s.contains(EffectSet::SUSPEND));
        assert!(!s.contains(EffectSet::INTERRUPT));
    }

    #[test]
    fn effect_set_union() {
        let a = EffectSet::from_bits(EffectSet::SUSPEND);
        let b = EffectSet::from_bits(EffectSet::INTERRUPT);
        let u = a.union(b);
        assert!(u.contains(EffectSet::SUSPEND));
        assert!(u.contains(EffectSet::INTERRUPT));
        assert!(!u.contains(EffectSet::DIVERGE));
    }

    #[test]
    fn effect_set_intersect() {
        let a = EffectSet::from_bits(EffectSet::SUSPEND | EffectSet::DIVERGE);
        let b = EffectSet::from_bits(EffectSet::DIVERGE | EffectSet::ALLOC);
        let i = a.intersect(b);
        assert!(i.contains(EffectSet::DIVERGE));
        assert!(!i.contains(EffectSet::SUSPEND));
        assert!(!i.contains(EffectSet::ALLOC));
    }

    #[test]
    fn effect_set_contains_bit() {
        let e = EffectSet::from_bits(EffectSet::SUSPEND | EffectSet::MMIO);
        assert!(e.contains(EffectSet::SUSPEND));
        assert!(e.contains(EffectSet::MMIO));
        assert!(!e.contains(EffectSet::INTERRUPT));
        assert!(!e.contains(EffectSet::DIVERGE));
        assert!(!e.contains(EffectSet::ALLOC));
    }

    #[test]
    fn effect_set_is_empty_true_when_no_bits() {
        assert!(EffectSet::empty().is_empty());
    }

    #[test]
    fn effect_set_is_empty_false_when_bits_set() {
        assert!(!EffectSet::from_bits(EffectSet::SUSPEND).is_empty());
    }

    // ---- CapSet -----------------------------------------------------------

    #[test]
    fn cap_set_empty() {
        let c = CapSet::empty();
        assert!(c.is_empty());
    }

    #[test]
    fn cap_set_subset_of_identical() {
        let a = CapSet::from_bits(CapSet::WRITE | CapSet::SUSPENDABLE);
        assert!(a.subset_of(a));
    }

    #[test]
    fn cap_set_subset_of_strict() {
        let req = CapSet::from_bits(CapSet::WRITE);
        let grants = CapSet::from_bits(CapSet::WRITE | CapSet::SUSPENDABLE);
        assert!(req.subset_of(grants));
    }

    #[test]
    fn cap_set_not_subset_when_missing() {
        let req = CapSet::from_bits(CapSet::WRITE | CapSet::COPYABLE);
        let grants = CapSet::from_bits(CapSet::WRITE);
        assert!(!req.subset_of(grants));
    }

    #[test]
    fn cap_set_subset_of_empty_grants() {
        let req = CapSet::from_bits(CapSet::WRITE);
        let grants = CapSet::empty();
        assert!(!req.subset_of(grants));
    }

    #[test]
    fn cap_set_empty_is_subset_of_anything() {
        assert!(CapSet::empty().subset_of(CapSet::empty()));
        assert!(CapSet::empty().subset_of(CapSet::from_bits(CapSet::WRITE)));
    }

    #[test]
    fn cap_set_union() {
        let a = CapSet::from_bits(CapSet::WRITE);
        let b = CapSet::from_bits(CapSet::SUSPENDABLE);
        let u = a.union(b);
        assert!(u.contains(CapSet::WRITE));
        assert!(u.contains(CapSet::SUSPENDABLE));
    }

    #[test]
    fn cap_set_contains() {
        let c = CapSet::from_bits(CapSet::COPYABLE | CapSet::BORROW_LIVE);
        assert!(c.contains(CapSet::COPYABLE));
        assert!(c.contains(CapSet::BORROW_LIVE));
        assert!(!c.contains(CapSet::WRITE));
        assert!(!c.contains(CapSet::BOUNDED_STACK));
    }

    // ---- High -------------------------------------------------------------

    #[test]
    fn high_zero() {
        assert_eq!(High::ZERO, High::Slots(0));
        assert!(!High::ZERO.is_top());
        assert!(High::ZERO.is_finite());
    }

    #[test]
    fn high_top_is_top() {
        assert!(High::Top.is_top());
        assert!(!High::Top.is_finite());
    }

    #[test]
    fn high_top_exceeds_everything() {
        assert!(High::Top.exceeds(High::Slots(0)));
        assert!(High::Top.exceeds(High::Slots(u32::MAX)));
    }

    #[test]
    fn high_nothing_exceeds_top_ceiling() {
        assert!(!High::Slots(100).exceeds(High::Top));
        assert!(!High::Top.exceeds(High::Top));
    }

    #[test]
    fn high_slots_exceeds_when_greater() {
        assert!(High::Slots(10).exceeds(High::Slots(5)));
        assert!(!High::Slots(5).exceeds(High::Slots(10)));
        assert!(!High::Slots(10).exceeds(High::Slots(10)));
    }

    #[test]
    fn high_add_positive() {
        assert_eq!(High::Slots(5).add_offset(3), High::Slots(8));
    }

    #[test]
    fn high_add_negative() {
        assert_eq!(High::Slots(10).add_offset(-3), High::Slots(7));
    }

    #[test]
    fn high_add_negative_saturates_zero() {
        assert_eq!(High::Slots(3).add_offset(-5), High::Slots(0));
    }

    #[test]
    fn high_add_top_stays_top() {
        assert_eq!(High::Top.add_offset(100), High::Top);
        assert_eq!(High::Top.add_offset(-100), High::Top);
    }

    #[test]
    fn high_add_overflow_to_top() {
        assert_eq!(High::Slots(u32::MAX).add_offset(1), High::Top);
    }

    // ---- StackBound -------------------------------------------------------

    #[test]
    fn stack_bound_identity() {
        assert_eq!(
            StackBound::ID,
            StackBound {
                net: 0,
                high: High::Slots(0)
            }
        );
    }

    #[test]
    fn stack_bound_compose_basic() {
        // dup: net=+1, high=+1
        let dup = StackBound {
            net: 1,
            high: High::Slots(1),
        };
        // drop: net=-1, high=0
        let drop = StackBound {
            net: -1,
            high: High::Slots(0),
        };
        // dup drop: net=0, high=1 (peaked at dup)
        let r = dup.compose(drop);
        assert_eq!(r.net, 0);
        assert_eq!(r.high, High::Slots(1));
    }

    #[test]
    fn stack_bound_compose_deep() {
        // dup dup drop swap: net = +1+1-1+0 = +1
        // high = max(1, 1+1=2, (1+1)+(-1)=1, (1+1+-1)+0=1) = max(1,2,1,1) = 2
        let dup = StackBound {
            net: 1,
            high: High::Slots(1),
        };
        let swap = StackBound {
            net: 0,
            high: High::Slots(0),
        };
        let r = dup
            .compose(dup)
            .compose(StackBound {
                net: -1,
                high: High::Slots(0),
            })
            .compose(swap);
        assert_eq!(r.net, 1);
        assert_eq!(r.high, High::Slots(2));
    }

    #[test]
    fn stack_bound_compose_associativity() {
        let a = StackBound {
            net: 2,
            high: High::Slots(3),
        };
        let b = StackBound {
            net: -1,
            high: High::Slots(2),
        };
        let c = StackBound {
            net: 3,
            high: High::Slots(1),
        };

        let left = a.compose(b).compose(c);
        let right = a.compose(b.compose(c));
        assert_eq!(left, right, "compose must be associative");
    }

    #[test]
    fn stack_bound_compose_identity_left() {
        let x = StackBound {
            net: 3,
            high: High::Slots(7),
        };
        assert_eq!(StackBound::ID.compose(x), x);
    }

    #[test]
    fn stack_bound_compose_identity_right() {
        let x = StackBound {
            net: 3,
            high: High::Slots(7),
        };
        assert_eq!(x.compose(StackBound::ID), x);
    }

    #[test]
    fn stack_bound_top_absorbs() {
        let finite = StackBound {
            net: 5,
            high: High::Slots(10),
        };
        let top = StackBound {
            net: 0,
            high: High::Top,
        };

        // finite ∘ top
        let r = finite.compose(top);
        assert_eq!(r.net, 5);
        assert_eq!(r.high, High::Top);

        // top ∘ finite
        let r = top.compose(finite);
        assert_eq!(r.net, 5);
        assert_eq!(r.high, High::Top);
    }

    #[test]
    fn stack_bound_top_absorbs_both_sides() {
        let a = StackBound {
            net: 1,
            high: High::Top,
        };
        let b = StackBound {
            net: 2,
            high: High::Top,
        };
        let r = a.compose(b);
        assert_eq!(r.net, 3);
        assert_eq!(r.high, High::Top);
    }

    #[test]
    fn stack_bound_branch_max() {
        let t = StackBound {
            net: 0,
            high: High::Slots(5),
        };
        let f = StackBound {
            net: 0,
            high: High::Slots(10),
        };
        let r = t.branch_max(f);
        assert_eq!(r.net, 0);
        assert_eq!(r.high, High::Slots(10));
    }

    #[test]
    fn stack_bound_branch_max_top_wins() {
        let t = StackBound {
            net: 2,
            high: High::Top,
        };
        let f = StackBound {
            net: 2,
            high: High::Slots(10),
        };
        let r = t.branch_max(f);
        assert_eq!(r.net, 2);
        assert_eq!(r.high, High::Top);
    }

    // The equal-net invariant is a debug_assert!, which compiles out under
    // --release — the profile CI tests with — so the panic only exists in
    // debug builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "branch_max requires equal net")]
    fn stack_bound_branch_max_unequal_net_panics() {
        let t = StackBound {
            net: 1,
            high: High::Slots(5),
        };
        let f = StackBound {
            net: 2,
            high: High::Slots(5),
        };
        let _ = t.branch_max(f);
    }

    #[test]
    fn stack_bound_wire_u32_finite() {
        let sb = StackBound {
            net: 3,
            high: High::Slots(42),
        };
        assert_eq!(sb.wire_u32(), 42);
    }

    #[test]
    fn stack_bound_wire_u32_top() {
        let sb = StackBound {
            net: 0,
            high: High::Top,
        };
        assert_eq!(sb.wire_u32(), 0xFFFF_FFFF);
    }

    #[test]
    fn stack_bound_wire_u32_zero() {
        let sb = StackBound::ID;
        assert_eq!(sb.wire_u32(), 0);
    }

    // ---- Context ----------------------------------------------------------

    #[test]
    fn context_default() {
        let ctx = Context::default();
        assert!(ctx.grants.is_empty());
        assert!(ctx.forbids.is_empty());
        assert_eq!(ctx.ceiling, High::Top);
    }

    #[test]
    fn context_new() {
        let grants = CapSet::from_bits(CapSet::WRITE);
        let forbids = EffectSet::from_bits(EffectSet::SUSPEND);
        let ceiling = High::Slots(256);
        let ctx = Context::new(grants, forbids, ceiling);
        assert_eq!(ctx.grants, grants);
        assert_eq!(ctx.forbids, forbids);
        assert_eq!(ctx.ceiling, ceiling);
    }

    // ---- shift_high -------------------------------------------------------

    #[test]
    fn shift_high_positive() {
        assert_eq!(shift_high(5, High::Slots(10)), High::Slots(15));
    }

    #[test]
    fn shift_high_negative() {
        assert_eq!(shift_high(-3, High::Slots(10)), High::Slots(7));
    }

    #[test]
    fn shift_high_negative_saturate() {
        assert_eq!(shift_high(-10, High::Slots(5)), High::Slots(0));
    }

    #[test]
    fn shift_high_top_stays_top() {
        assert_eq!(shift_high(100, High::Top), High::Top);
        assert_eq!(shift_high(-100, High::Top), High::Top);
    }

    #[test]
    fn shift_high_overflow() {
        assert_eq!(shift_high(1, High::Slots(u32::MAX)), High::Top);
    }

    // -----------------------------------------------------------------------
    // abi_hash
    // -----------------------------------------------------------------------

    #[test]
    fn abi_hash_stable() {
        // Same inputs must produce the same hash.
        let h1 = abi_hash(
            b"i64",
            b"i64",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        let h2 = abi_hash(
            b"i64",
            b"i64",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        assert_eq!(h1, h2);
    }

    #[test]
    fn abi_hash_changes_on_performs() {
        let base = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        let with_suspend = abi_hash(
            b"",
            b"",
            EffectSet::from_bits(EffectSet::SUSPEND),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        assert_ne!(base, with_suspend);
    }

    #[test]
    fn abi_hash_changes_on_bound() {
        let base = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        let bigger = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound {
                net: 1,
                high: High::Slots(5),
            },
            8,
        );
        assert_ne!(base, bigger);
    }

    #[test]
    fn abi_hash_changes_on_slot_bytes() {
        let base = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        let different = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            4,
        );
        assert_ne!(base, different);
    }

    #[test]
    fn abi_hash_changes_on_requires() {
        let base = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        let with_req = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::from_bits(CapSet::WRITE),
            StackBound::ID,
            8,
        );
        assert_ne!(base, with_req);
    }

    #[test]
    fn abi_hash_changes_on_sig_in() {
        let base = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        let with_sig = abi_hash(
            b"i64",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        assert_ne!(base, with_sig);
    }

    #[test]
    fn abi_hash_changes_on_sig_out() {
        let base = abi_hash(
            b"",
            b"",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        let with_sig = abi_hash(
            b"",
            b"bool",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        assert_ne!(base, with_sig);
    }

    /// Golden value for a fixed input — changing this constant is an ABI break.
    /// If you need to change the hash algorithm, bump ABI_CONTRACT_VERSION.
    #[test]
    fn abi_hash_golden_v1() {
        let h = abi_hash(
            b"i64",
            b"i64",
            EffectSet::empty(),
            CapSet::empty(),
            StackBound::ID,
            8,
        );
        assert_eq!(
            h, 14985849781704156055,
            "ABI_HASH_GOLDEN_V1 changed — this is an ABI break. Bump ABI_CONTRACT_VERSION."
        );
    }
}
