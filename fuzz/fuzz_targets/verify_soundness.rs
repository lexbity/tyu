//! Fuzz of the interval engine's soundness property (slice P5, Q9): the
//! fuzz input seeds a deterministic generator of straight-line OEL-style
//! programs; each program is discharged over its input intervals and then run
//! concretely (wrapping i64, subtype cast trap) over an exhaustive tiny
//! domain. A **false discharge** (abstract `DefTrue` but a concrete witness
//! violates) panics — libFuzzer minimizes to the offending bytes.
//!
//! This is a mirror of `crates/verifier/tests/soundness_differential.rs`
//! (the CI gate, FR-20); the fuzz target widens the search beyond the fixed
//! seed.

#![no_main]

use libfuzzer_sys::fuzz_target;

use ir::{CmpKind, OpKind, TypeId};
use verifier::interval::{Interval, Tri, eval_in_range};
use verifier::interp::{Origin, Slot, State};

const SUB_ID: u8 = 2;
const SUB_LO: i64 = 0;
const SUB_HI: i64 = 100;

fn sr() -> &'static verifier::interp::SubtypeRange<'static> {
    &|tid| if tid.0 == SUB_ID { Some((SUB_LO, SUB_HI)) } else { None }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A compact deterministic generator (well-formed by construction: it tracks
/// the stack height and only emits valid pops).
fn gen_program(seed: &[u8], ops: &mut Vec<OpKind>, domains: &mut Vec<(i64, i64)>) {
    let mut rng = Rng(0x5eed_5eed_5eed_5eed ^ seed.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    }));
    let n_inputs = 1 + (rng.below(3) as usize);
    for _ in 0..n_inputs {
        let lo = (rng.next() % 8) as i64 - 4;
        let hi = lo + (rng.next() % 8) as i64;
        domains.push((lo, hi));
    }
    let mut depth = n_inputs;
    let len = 1 + (rng.below(8) as usize);
    for _ in 0..len {
        match rng.below(10) {
            0 => {
                ops.push(OpKind::ConstI64(((rng.next() % 11) as i64) - 5));
                depth += 1;
            }
            1..=3 => {
                if depth >= 2 {
                    ops.push(match rng.below(3) {
                        0 => OpKind::AddI64,
                        1 => OpKind::SubI64,
                        _ => OpKind::MulI64,
                    });
                    depth -= 1;
                }
            }
            4 => {
                if depth >= 1 {
                    ops.push(OpKind::Dup { ty: TypeId(0) });
                    depth += 1;
                }
            }
            5 => {
                if depth >= 2 {
                    ops.push(OpKind::Drop { ty: TypeId(0) });
                    depth -= 1;
                }
            }
            6 => {
                if depth >= 2 {
                    ops.push(OpKind::Swap { a: TypeId(0), b: TypeId(0) });
                }
            }
            7 => {
                if depth >= 1 {
                    ops.push(OpKind::LocalSet { slot: 0, ty: TypeId(0) });
                    depth -= 1;
                }
            }
            8 => {
                ops.push(OpKind::LocalGet { slot: 0, ty: TypeId(0) });
                depth += 1;
            }
            _ => {
                if depth >= 2 {
                    ops.push(OpKind::Cmp { out: TypeId(1), kind: CmpKind::Eq });
                    depth -= 1;
                }
            }
        }
    }
}

/// Concrete reference (wrapping; the subtype cast traps out-of-range).
fn concrete(ops: &[OpKind], inputs: &[i64]) -> Option<i64> {
    let mut stack: Vec<i64> = inputs.to_vec();
    let mut locals = [0i64; 4];
    for op in ops {
        match op {
            OpKind::ConstI64(v) => stack.push(*v),
            OpKind::AddI64 => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.wrapping_add(b));
            }
            OpKind::SubI64 => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.wrapping_sub(b));
            }
            OpKind::MulI64 => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(a.wrapping_mul(b));
            }
            OpKind::Dup { .. } => {
                let v = *stack.last()?;
                stack.push(v);
            }
            OpKind::Drop { .. } => {
                let _ = stack.pop()?;
            }
            OpKind::Swap { .. } => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(b);
                stack.push(a);
            }
            OpKind::Cmp { kind, .. } => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                let t = match kind {
                    CmpKind::Lt => a < b,
                    CmpKind::Le => a <= b,
                    CmpKind::Gt => a > b,
                    CmpKind::Ge => a >= b,
                    CmpKind::Eq => a == b,
                    CmpKind::Ne => a != b,
                };
                stack.push(if t { 1 } else { 0 });
            }
            OpKind::LocalGet { slot, .. } => {
                let v = locals[*slot as usize % 4];
                stack.push(v);
            }
            OpKind::LocalSet { slot, .. } => {
                let v = stack.pop()?;
                locals[*slot as usize % 4] = v;
            }
            OpKind::Cast { to, .. } => {
                let v = stack.pop()?;
                if to.0 == SUB_ID && !(SUB_LO <= v && v <= SUB_HI) {
                    return None;
                }
                stack.push(v);
            }
            _ => return None,
        }
    }
    stack.last().copied()
}

/// Enumerate the input-domain cube.
fn enumerate(domains: &[(i64, i64)], at: usize, acc: &mut Vec<i64>, out: &mut Vec<Vec<i64>>) {
    if at == domains.len() {
        out.push(acc.clone());
        return;
    }
    let (lo, hi) = domains[at];
    for v in lo..=hi {
        acc.push(v);
        enumerate(domains, at + 1, acc, out);
        acc.pop();
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let mut ops = Vec::new();
    let mut domains = Vec::new();
    gen_program(data, &mut ops, &mut domains);

    // Abstract discharge over the domain intervals.
    let mut st = State::callee_entry(domains.len(), 4);
    st.stack.clear();
    for &(lo, hi) in &domains {
        st.stack.push(Slot {
            iv: Interval::Range { lo, hi },
            origin: Origin::Arg(0),
        });
    }
    for op in &ops {
        st.step(op, sr());
    }
    let tri = eval_in_range(st.top_interval(), SUB_LO, SUB_HI);
    if tri != Tri::DefTrue {
        return; // open/provably-failing: the check stays (conservative)
    }

    // Any concrete witness that violates is a FALSE DISCHARGE — the point of
    // this target.
    // Guard the total input count: the domains are tiny (≤ 8 values each), so
    // the cube is ≤ 8^3 = 512 points.
    let mut cube = Vec::new();
    enumerate(&domains, 0, &mut Vec::new(), &mut cube);
    for input in &cube {
        match concrete(&ops, input) {
            None => panic!("false discharge: cast trap on {input:?}"),
            Some(v) if !(SUB_LO <= v && v <= SUB_HI) => {
                panic!("false discharge: {input:?} -> {v} out of [{SUB_LO},{SUB_HI}]")
            }
            _ => {}
        }
    }
});