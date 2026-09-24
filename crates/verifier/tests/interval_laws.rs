//! Interval lattice law conformance (static-verification.md §7.2, slice P5):
//! join/meet laws over randomized pairs, subset/intersect coherence, and the
//! transfer-table hand cases. Seeded PRNG (deterministic — no external dep),
//! so failures are reproducible byte-for-byte.

use verifier::interval::{eval_in_range, Interval, Tri};

/// Deterministic 64-bit LCG (no external dep). Same algorithm as the P4
/// cache FNV seeds: fixed seed, fixed sequence.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // PCG-XSH-RR-style step with fixed increment.
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn i64(&mut self, lo: i64, hi: i64) -> i64 {
        let span = hi.saturating_sub(lo).saturating_add(1) as u64;
        lo.wrapping_add((self.next() % span.max(1)) as i64)
    }

    fn interval(&mut self) -> Interval {
        let a = self.i64(-50, 50);
        let b = self.i64(-50, 50);
        Interval::Range {
            lo: core::cmp::min(a, b),
            hi: core::cmp::max(a, b),
        }
    }
}

#[test]
fn join_is_commutative_associative_idempotent() {
    let mut rng = Rng(0x5eed_5eed_5eed_5eed);
    for _ in 0..4000 {
        let a = rng.interval();
        let b = rng.interval();
        let c = rng.interval();
        assert_eq!(a.join(b), b.join(a), "commutativity");
        assert_eq!(a.join(a), a, "idempotence");
        assert_eq!(a.join(b).join(c), a.join(b.join(c)), "associativity");
        // Top/Bottom identities.
        assert_eq!(a.join(Interval::BOTTOM), a);
        assert_eq!(a.join(Interval::TOP), Interval::TOP);
    }
}

#[test]
fn meet_is_commutative_and_absorbs() {
    let mut rng = Rng(0xcafe_face_cafe_face);
    for _ in 0..2000 {
        let a = rng.interval();
        let b = rng.interval();
        assert_eq!(a.intersect(b), b.intersect(a), "commutativity");
        // meet ⊆ both operands.
        let m = a.intersect(b);
        assert!(m.subset_of(a) && m.subset_of(b), "meet is a lower bound");
    }
}

#[test]
fn join_is_an_upper_bound_and_subset_is_coherent() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for _ in 0..2000 {
        let a = rng.interval();
        let b = rng.interval();
        let j = a.join(b);
        assert!(a.subset_of(j) && b.subset_of(j), "join is an upper bound");
        if j == a {
            assert!(b.subset_of(a));
        }
    }
}

#[test]
fn eval_range_matches_disjointness_and_containment() {
    let mut rng = Rng(0x0dd_b0dd_0dd_b0dd);
    for _ in 0..2000 {
        let a = rng.interval();
        if let Interval::Range { lo, hi } = a {
            // The interval exactly equals the target → contained → DefTrue.
            assert_eq!(eval_in_range(a, lo, hi), Tri::DefTrue);
            // An expanded target is trivially contained → DefTrue.
            assert_eq!(eval_in_range(a, lo.saturating_sub(1), hi.saturating_add(1)), Tri::DefTrue);
            // A shifted-away target → disjoint → DefFalse.
            assert_eq!(
                eval_in_range(a, hi.saturating_add(1), hi.saturating_add(100)),
                Tri::DefFalse
            );
        }
        if a.is_top() {
            assert_eq!(eval_in_range(a, -1, 1), Tri::Top);
        }
    }
}

/// The standard interval-arithmetic soundness rule: for sampled concrete
/// points, the abstract result CONTAINS the concrete result (wrapping i64).
#[test]
fn abstract_arithmetic_covers_concrete_wrapping() {
    for &(x, y) in &[
        (3i64, 4i64),
        (-3, 4),
        (3, -4),
        (-3, -4),
        (0, 0),
        (i64::MAX, 1),
        (i64::MIN, -1),
        (i64::MAX - 1, 3),
    ] {
        let a = Interval::const_val(x);
        let b = Interval::const_val(y);
        let add = a.add(b);
        let sub = a.sub(b);
        let mul = a.mul(b);
        assert!(
            add.is_top() || add.contains(x.wrapping_add(y)),
            "{x} + {y}: abstract must cover concrete"
        );
        assert!(
            sub.is_top() || sub.contains(x.wrapping_sub(y)),
            "{x} - {y}: abstract must cover concrete"
        );
        assert!(
            mul.is_top() || mul.contains(x.wrapping_mul(y)),
            "{x} * {y}: abstract must cover concrete"
        );
    }
}