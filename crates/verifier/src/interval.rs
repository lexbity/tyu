//! The interval lattice (static-verification.md §7.2, slice P5).
//!
//! Abstract values for the in-tree discharger:
//!
//! ```text
//! Interval = ⊥ | [lo, hi] | ⊤        (standard interval lattice)
//! Origin   = Arg(i) | Local(i) | Computed | ⊤   (identity lattice — P6's E3312)
//! State    = { stack: slot → Interval×Origin, locals: id → Interval×Origin }
//! ```
//!
//! Soundness contract (Q4/§7.2): **all bound arithmetic is i64 `checked_*`;
//! any overflow in a bound computation yields `⊤`** — sound against the
//! wrapping two's-complement runtime semantics, since `⊤` (or anything wider
//! than the true bound) never discharges a `⊤`-unsafe site. `Bottom` denotes
//! an unreachable value (empty set); it never discharges and never marks
//! `provably_failing` (it is not "always failing" — it is "never reached").
//!
//! `Tri` is the abstract three-valued boolean (`DefTrue` / `DefFalse` / `Top`)
//! used for obligation heads: a head discharging requires `DefTrue` (Q6/Q4).

/// The abstract three-valued boolean for obligation heads (§7.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tri {
    /// The predicate is provably true over the whole interval.
    DefTrue,
    /// The predicate is provably false over the whole interval — never a
    /// discharge; recorded as `provably_failing` with the check retained.
    DefFalse,
    /// Unknown / mixed — the obligation stays open.
    Top,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interval {
    /// Empty — no concrete value can be here (unreachable).
    Bottom,
    /// The closed range `lo..=hi`, with `lo <= hi`.
    Range { lo: i64, hi: i64 },
    /// The whole i64 domain.
    Top,
}

impl Interval {
    pub const BOTTOM: Interval = Interval::Bottom;
    pub const TOP: Interval = Interval::Top;

    /// The singleton interval of a compile-time constant.
    pub fn const_val(v: i64) -> Interval {
        Interval::Range { lo: v, hi: v }
    }

    pub fn is_bottom(self) -> bool {
        matches!(self, Interval::Bottom)
    }

    pub fn is_top(self) -> bool {
        matches!(self, Interval::Top)
    }

    /// The domain of a subtype declaration (a compile-time fact).
    pub fn range(lo: i64, hi: i64) -> Interval {
        debug_assert!(lo <= hi);
        Interval::Range { lo, hi }
    }

    /// Hull join (`v1 ∨ v2`, §7.2): `Bottom` is the identity, `⊤` absorbs,
    /// two ranges hull to `[min lo, max hi]`. Commutative, associative,
    /// idempotent (tested in `interval_laws.rs`).
    pub fn join(self, other: Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, x) | (x, Interval::Bottom) => x,
            (Interval::Top, _) | (_, Interval::Top) => Interval::Top,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                Interval::Range {
                    lo: core::cmp::min(a, c),
                    hi: core::cmp::max(b, d),
                }
            }
        }
    }

    /// Intersection (`meet`): empty when the ranges do not overlap. `⊤ ∩ x =
    /// x`; `⊥` absorbs.
    pub fn intersect(self, other: Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            (Interval::Top, x) | (x, Interval::Top) => x,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                let lo = core::cmp::max(a, c);
                let hi = core::cmp::min(b, d);
                if lo > hi {
                    Interval::Bottom
                } else {
                    Interval::Range { lo, hi }
                }
            }
        }
    }

    /// `self ⊆ other` (pointwise set inclusion; `⊤` is the whole domain,
    /// `⊥` the empty set). `Bottom ⊆ x` always; `x ⊆ Top` always.
    pub fn subset_of(self, other: Interval) -> bool {
        match (self, other) {
            (Interval::Bottom, _) => true,
            (_, Interval::Top) => true,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                c <= a && b <= d
            }
            // Range ⊄ Bottom; Top ⊄ anything proper; Bottom ⊄ nothing (handled);
            // Top ⊆ Top handled above.
            _ => false,
        }
    }

    /// Definitive membership of a concrete value.
    pub fn contains(self, v: i64) -> bool {
        match self {
            Interval::Bottom => false,
            Interval::Range { lo, hi } => lo <= v && v <= hi,
            Interval::Top => true,
        }
    }

    /// Whether the interval is a single point.
    pub fn is_point(self) -> bool {
        match self {
            Interval::Range { lo, hi } => lo == hi,
            _ => false,
        }
    }

    // --- arithmetic (Q4/§7.2: checked; overflow → ⊤) ---

    /// `[lo1, hi1] + [lo2, hi2] = [lo1+lo2, hi1+hi2]`; a bound overflow in
    /// i64 is `⊤` (sound versus wrapping: `⊤` never discharges).
    pub fn add(self, other: Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            (Interval::Top, _) | (_, Interval::Top) => Interval::Top,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                let (Some(lo), Some(hi)) = (a.checked_add(c), b.checked_add(d)) else {
                    return Interval::Top;
                };
                Interval::Range { lo, hi }
            }
        }
    }

    /// `[lo1, hi1] - [lo2, hi2] = [lo1-hi2, hi1-lo2]`.
    pub fn sub(self, other: Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            (Interval::Top, _) | (_, Interval::Top) => Interval::Top,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                let (Some(lo), Some(hi)) = (a.checked_sub(d), b.checked_sub(c)) else {
                    return Interval::Top;
                };
                Interval::Range { lo, hi }
            }
        }
    }

    /// Interval multiplication by the four-corner tableau (§7.2):
    /// `[min(a·c, a·d, b·c, b·d), max(...)]`. Any `i64` overflow on a corner
    /// makes the result `⊤` — sound against wrapping (concrete mul wraps, so
    /// an overflowing corner could produce any wrapped value; `⊤` never
    /// discharges). All 9 sign combinations are unit-tested by hand.
    pub fn mul(self, other: Interval) -> Interval {
        match (self, other) {
            (Interval::Bottom, _) | (_, Interval::Bottom) => Interval::Bottom,
            (Interval::Top, _) | (_, Interval::Top) => Interval::Top,
            (Interval::Range { lo: a, hi: b }, Interval::Range { lo: c, hi: d }) => {
                let corners = [
                    a.checked_mul(c),
                    a.checked_mul(d),
                    b.checked_mul(c),
                    b.checked_mul(d),
                ];
                match corners {
                    [Some(w), Some(x), Some(y), Some(z)] => {
                        Interval::Range {
                            lo: w.min(x).min(y).min(z),
                            hi: w.max(x).max(y).max(z),
                        }
                    }
                    _ => Interval::Top,
                }
            }
        }
    }

    /// The abstract result of a narrowing cast to a subtype range:
    /// `iv ∩ [lo, hi]`. An empty intersection is `Bottom` — the abstract
    /// successor domain of a cast that (provably) always traps (the runtime
    /// check, retained when the site is open, stops control flow).
    pub fn cast_narrow(self, lo: i64, hi: i64) -> Interval {
        self.intersect(Interval::range(lo, hi))
    }
}

/// Evaluate `InRange(iv, lo, hi)` to the three-valued head (Q4/§7.2).
///
/// - interval ⊆ `[lo,hi]` (non-`Bottom`) → `DefTrue` (discharge);
/// - interval ∩ `[lo,hi] == ∅` (`Bottom` input already excluded) → `DefFalse`
///   (recorded `provably_failing`; the check is retained);
/// - otherwise → `Top` (open).
///
/// A `Bottom` value is *unreachable*, not "always failing" — it resolves
/// `Top` so downstream sites in unreachable code conservatively keep their
/// checks.
pub fn eval_in_range(iv: Interval, lo: i64, hi: i64) -> Tri {
    match iv {
        Interval::Bottom => Tri::Top,
        Interval::Top => Tri::Top,
        Interval::Range { lo: a, hi: b } => {
            if a >= lo && b <= hi {
                Tri::DefTrue
            } else if b < lo || a > hi {
                Tri::DefFalse
            } else {
                Tri::Top
            }
        }
    }
}

/// Interpret an interval as the abstract bool domain (`[0,0]`/`[1,1]`/
/// mixed). A non-bool-typed interval is treated conservatively as
/// `Top` — this function is only fed intervals the typechecker proved bool.
pub fn tri_from_bool_iv(iv: Interval) -> Tri {
    match iv {
        Interval::Range { lo, hi } if lo == 0 && hi == 0 => Tri::DefFalse,
        Interval::Range { lo, hi } if lo == 1 && hi == 1 => Tri::DefTrue,
        _ => Tri::Top,
    }
}

/// The bool interval contribution of a comparison result.
pub fn bool_iv(t: Tri) -> Interval {
    match t {
        Tri::DefTrue => Interval::const_val(1),
        Tri::DefFalse => Interval::const_val(0),
        Tri::Top => Interval::Range { lo: 0, hi: 1 },
    }
}

pub fn tri_not(t: Tri) -> Tri {
    match t {
        Tri::DefTrue => Tri::DefFalse,
        Tri::DefFalse => Tri::DefTrue,
        Tri::Top => Tri::Top,
    }
}

pub fn tri_and(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Tri::DefTrue, x) | (x, Tri::DefTrue) => x,
        (Tri::DefFalse, _) | (_, Tri::DefFalse) => Tri::DefFalse,
        _ => Tri::Top,
    }
}

pub fn tri_or(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Tri::DefFalse, x) | (x, Tri::DefFalse) => x,
        (Tri::DefTrue, _) | (_, Tri::DefTrue) => Tri::DefTrue,
        _ => Tri::Top,
    }
}

/// The abstract comparison result `a rel b` over intervals (§7.2 Cmp row):
/// `DefTrue` when the relation provably holds for every pair, `DefFalse` when
/// it provably never holds, `Top` otherwise. `Bottom` operands are
/// conservative `Top` (unreachable).
pub fn tri_cmp(a: Interval, b: Interval, kind: ir::CmpKind) -> Tri {
    let ord = match (a, b) {
        (Interval::Bottom, _) | (_, Interval::Bottom) => return Tri::Top,
        (Interval::Top, _) | (_, Interval::Top) => {
            // A ⊤ operand can pair with anything — nothing is provable.
            return Tri::Top;
        }
        (Interval::Range { lo: x, hi: y }, Interval::Range { lo: u, hi: v }) => (x, y, u, v),
    };
    let (a_lo, a_hi, b_lo, b_hi) = ord;
    match kind {
        ir::CmpKind::Lt => {
            if a_hi < b_lo {
                Tri::DefTrue
            } else if a_lo >= b_hi {
                Tri::DefFalse
            } else {
                Tri::Top
            }
        }
        ir::CmpKind::Le => {
            if a_hi <= b_lo {
                Tri::DefTrue
            } else if a_lo > b_hi {
                Tri::DefFalse
            } else {
                Tri::Top
            }
        }
        ir::CmpKind::Gt => {
            if a_lo > b_hi {
                Tri::DefTrue
            } else if a_hi <= b_lo {
                Tri::DefFalse
            } else {
                Tri::Top
            }
        }
        ir::CmpKind::Ge => {
            if a_lo >= b_hi {
                Tri::DefTrue
            } else if a_hi < b_lo {
                Tri::DefFalse
            } else {
                Tri::Top
            }
        }
        ir::CmpKind::Eq => {
            if a_lo == a_hi && b_lo == b_hi && a_lo == b_lo {
                Tri::DefTrue
            } else if a_hi < b_lo || a_lo > b_hi {
                Tri::DefFalse
            } else {
                Tri::Top
            }
        }
        ir::CmpKind::Ne => {
            if a_lo == a_hi && b_lo == b_hi && a_lo == b_lo {
                Tri::DefFalse
            } else if a_hi < b_lo || a_lo > b_hi {
                Tri::DefTrue
            } else {
                Tri::Top
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn r(lo: i64, hi: i64) -> Interval {
        Interval::Range { lo, hi }
    }

    #[test]
    fn lattice_identities() {
        // Top/Bottom identities for join and meet.
        assert_eq!(Interval::BOTTOM.join(r(1, 2)), r(1, 2));
        assert_eq!(r(1, 2).join(Interval::BOTTOM), r(1, 2));
        assert_eq!(Interval::TOP.join(r(1, 2)), Interval::TOP);
        assert_eq!(r(1, 2).join(Interval::TOP), Interval::TOP);
        assert_eq!(Interval::TOP.intersect(r(1, 2)), r(1, 2));
        assert_eq!(r(1, 2).intersect(Interval::TOP), r(1, 2));
        assert_eq!(Interval::BOTTOM.intersect(r(1, 2)), Interval::BOTTOM);
    }

    #[test]
    fn join_hulls_and_subset() {
        assert_eq!(r(1, 3).join(r(7, 9)), r(1, 9));
        assert_eq!(r(-5, -1).join(r(0, 2)), r(-5, 2));
        assert!(r(1, 2).subset_of(r(0, 3)));
        assert!(!r(1, 4).subset_of(r(0, 3)));
        assert!(Interval::BOTTOM.subset_of(r(0, 3)));
        assert!(r(1, 2).subset_of(Interval::TOP));
        assert!(Interval::BOTTOM.subset_of(Interval::BOTTOM));
        assert!(r(1, 1).subset_of(r(1, 1)));
    }

    #[test]
    fn intersect_rules() {
        assert_eq!(r(1, 5).intersect(r(3, 9)), r(3, 5));
        assert_eq!(r(1, 5).intersect(r(6, 9)), Interval::BOTTOM);
        assert_eq!(r(1, 5).intersect(r(-2, 0)), Interval::BOTTOM);
        assert_eq!(r(1, 5).intersect(r(1, 5)), r(1, 5));
        // Touching at a point.
        assert_eq!(r(1, 5).intersect(r(5, 5)), r(5, 5));
    }

    #[test]
    fn add_sub_mul_basic() {
        assert_eq!(r(1, 3).add(r(4, 5)), r(5, 8));
        assert_eq!(r(-3, -1).add(r(1, 5)), r(-2, 4));
        assert_eq!(r(1, 3).sub(r(4, 5)), r(-4, -1));
        assert_eq!(r(-3, -1).mul(r(1, 5)), r(-15, -1));
        assert_eq!(r(1, 3).mul(r(1, 5)), r(1, 15));
        assert_eq!(r(-5, 3).mul(r(-2, 4)), r(-20, 12));
    }

    /// The 9 sign-combination multiplication tableau (hand-computed bounds).
    #[test]
    fn mul_sign_tableau_all_nine() {
        // both positive
        assert_eq!(r(2, 4).mul(r(3, 5)), r(6, 20));
        // left positive, right all-negative
        assert_eq!(r(2, 4).mul(r(-5, -3)), r(-20, -6));
        // left negative, right positive
        assert_eq!(r(-4, -2).mul(r(3, 5)), r(-20, -6));
        // both negative
        assert_eq!(r(-4, -2).mul(r(-5, -3)), r(6, 20));
        // left spans zero
        assert_eq!(r(-2, 3).mul(r(4, 5)), r(-10, 15));
        // right spans zero
        assert_eq!(r(4, 5).mul(r(-2, 3)), r(-10, 15));
        // both span zero
        assert_eq!(r(-2, 3).mul(r(-3, 4)), r(-9, 12));
        // one is a point
        assert_eq!(r(-2, 3).mul(r(4, 4)), r(-8, 12));
        assert_eq!(r(5, 5).mul(r(-2, 4)), r(-10, 20));
    }

    #[test]
    fn overflow_saturates_to_top() {
        assert_eq!(Interval::TOP.add(r(1, 1)), Interval::TOP);
        assert_eq!(r(i64::MAX - 1, i64::MAX).add(r(1, 2)), Interval::TOP);
        assert_eq!(r(i64::MIN + 1, i64::MIN).sub(r(2, 4)), Interval::TOP);
        assert_eq!(r(i64::MAX - 1, i64::MAX).mul(r(2, 2)), Interval::TOP);
        assert_eq!(r(i64::MIN, i64::MIN).mul(r(-1, -1)), Interval::TOP);
    }

    #[test]
    fn narrowing_cast_meet() {
        assert_eq!(r(50, 50).cast_narrow(0, 100), r(50, 50));
        assert_eq!(r(-5, 200).cast_narrow(0, 100), r(0, 100));
        assert_eq!(r(150, 150).cast_narrow(0, 100), Interval::BOTTOM);
        assert_eq!(Interval::TOP.cast_narrow(0, 100), r(0, 100));
    }

    #[test]
    fn eval_range_three_valued() {
        assert_eq!(eval_in_range(r(50, 50), 0, 100), Tri::DefTrue);
        assert_eq!(eval_in_range(r(0, 100), 0, 100), Tri::DefTrue);
        assert_eq!(eval_in_range(r(150, 150), 0, 100), Tri::DefFalse);
        assert_eq!(eval_in_range(r(-5, -1), 0, 100), Tri::DefFalse);
        assert_eq!(eval_in_range(r(50, 200), 0, 100), Tri::Top);
        assert_eq!(eval_in_range(Interval::TOP, 0, 100), Tri::Top);
        assert_eq!(eval_in_range(Interval::BOTTOM, 0, 100), Tri::Top);
    }

    #[test]
    fn cmp_table() {
        use ir::CmpKind::*;
        assert_eq!(tri_cmp(r(1, 2), r(5, 7), Lt), Tri::DefTrue);
        assert_eq!(tri_cmp(r(1, 2), r(0, 1), Lt), Tri::DefFalse);
        assert_eq!(tri_cmp(r(1, 3), r(2, 4), Lt), Tri::Top);
        assert_eq!(tri_cmp(r(6, 9), r(1, 2), Lt), Tri::DefFalse);
        assert_eq!(tri_cmp(r(1, 2), r(2, 3), Le), Tri::DefTrue);
        assert_eq!(tri_cmp(r(1, 2), r(3, 4), Ge), Tri::DefFalse); // 1,2 ≥ 3,4 never
        assert_eq!(tri_cmp(r(9, 9), r(1, 2), Gt), Tri::DefTrue);
        assert_eq!(tri_cmp(r(1, 2), r(5, 7), Ge), Tri::DefFalse);
        assert_eq!(tri_cmp(r(5, 5), r(5, 5), Eq), Tri::DefTrue);
        assert_eq!(tri_cmp(r(1, 5), r(5, 5), Eq), Tri::Top);
        assert_eq!(tri_cmp(r(6, 9), r(1, 2), Eq), Tri::DefFalse);
        assert_eq!(tri_cmp(r(5, 5), r(5, 5), Ne), Tri::DefFalse);
        assert_eq!(tri_cmp(r(6, 9), r(1, 2), Ne), Tri::DefTrue);
        assert_eq!(tri_cmp(Interval::TOP, r(1, 2), Lt), Tri::Top);
    }

    #[test]
    fn tri_bool_algebra() {
        assert_eq!(tri_and(Tri::DefTrue, Tri::DefTrue), Tri::DefTrue);
        assert_eq!(tri_and(Tri::DefTrue, Tri::DefFalse), Tri::DefFalse);
        assert_eq!(tri_and(Tri::DefFalse, Tri::Top), Tri::DefFalse);
        assert_eq!(tri_and(Tri::Top, Tri::Top), Tri::Top);
        assert_eq!(tri_or(Tri::DefTrue, Tri::Top), Tri::DefTrue);
        assert_eq!(tri_or(Tri::DefFalse, Tri::Top), Tri::Top);
        assert_eq!(tri_or(Tri::DefFalse, Tri::DefFalse), Tri::DefFalse);
        assert_eq!(tri_not(Tri::DefTrue), Tri::DefFalse);
        assert_eq!(tri_not(Tri::DefFalse), Tri::DefTrue);
        assert_eq!(tri_not(Tri::Top), Tri::Top);
        assert_eq!(bool_iv(Tri::DefTrue), Interval::const_val(1));
        assert_eq!(tri_from_bool_iv(Interval::const_val(0)), Tri::DefFalse);
        assert_eq!(tri_from_bool_iv(r(0, 1)), Tri::Top);
    }
}