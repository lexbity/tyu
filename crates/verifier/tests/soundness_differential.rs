//! Differential soundness harness for the interval engine (static-
//! verification.md Q9/P5, FR-20/NFR-3).
//!
//! A *concrete reference interpreter* for the OEL program shape evaluates the
//! engine's transfer functions (`verifier::interp::State::step`) against
//! actual wrapping-i64 execution:
//!
//! - **Discharged ⇒ no concrete witness violates** (a false `Discharged`
//!   fails CI);
//! - **DefFalse ⇒ every concrete witness violates** (feeds `provably_failing`)
//! - loop-CFG programs: the back-edge widening (FR-12) must never produce a
//!   false discharge.
//!
//! Exhaustive suites over crafted programs with inputs from tiny domains
//! (≤ 2^14 points), plus a deterministic seeded generator producing ≥ 10^5
//! random straight-line programs per run. The generator is seeded (fixed
//! seed), so a run is byte-reproducible across machines.

use ir::{Atom, BlockId, CmpKind, OpKind, Sig, Span, TypeId};
use verifier::interval::{eval_in_range, Interval, Tri};
use verifier::interp::{run_cfg, State, SubtypeRange};

/// The harness's single subtype: `TypeId(2)` ranges `0..=100` (mirrors the
/// `Percent` books example).
const SUB_ID: u8 = 2;
const SUB_LO: i64 = 0;
const SUB_HI: i64 = 100;

fn subtype_range_lookup(tid: TypeId) -> Option<(i64, i64)> {
    if tid.0 == SUB_ID {
        Some((SUB_LO, SUB_HI))
    } else {
        None
    }
}

fn sr() -> &'static SubtypeRange<'static> {
    &subtype_range_lookup
}

// ---------------------------------------------------------------------------
// Deterministic PRNG (no external dep)
// ---------------------------------------------------------------------------

pub struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn i64_between(&mut self, lo: i64, hi: i64) -> i64 {
        let span = hi.saturating_sub(lo).saturating_add(1) as u64;
        lo.wrapping_add((self.next() % span.max(1)) as i64)
    }
}

// ---------------------------------------------------------------------------
// Program model: straight-line ops over `n_inputs` inputs, ending with an
// `InRange` obligation on the final top of the abstract stack.
// ---------------------------------------------------------------------------

/// One straight-line program with its input domains and the final obligation.
#[derive(Debug)]
pub struct Program {
    pub n_inputs: usize,
    /// Per-input (lo, hi) concrete domains — the concrete enumeration is
    /// EXACTLY these intervals, so the exhaustive check is complete over the
    /// domain the discharge claims.
    pub domains: Vec<(i64, i64)>,
    pub ops: Vec<OpKind>,
    pub target: (i64, i64),
    /// When true, the obligation is on the PRE-cast value of the final
    /// `Cast` op (the cast's own site) — concrete runs trap on out-of-range.
    pub cast_site: bool,
}

/// Enumerate every combination of input values across the domains.
fn enumerate_inputs(domains: &[(i64, i64)], out: &mut Vec<Vec<i64>>) {
    fn rec(domains: &[(i64, i64)], at: usize, acc: &mut Vec<i64>, out: &mut Vec<Vec<i64>>) {
        if at == domains.len() {
            out.push(acc.clone());
            return;
        }
        let (lo, hi) = domains[at];
        for v in lo..=hi {
            acc.push(v);
            rec(domains, at + 1, acc, out);
            acc.pop();
        }
    }
    out.clear();
    rec(domains, 0, &mut Vec::new(), out);
}

// Concrete semantics ---------------------------------------------------------

/// Concrete result of one straight-line run: a value, or a trap (the cast
/// rejected an out-of-range value — the runtime trap).
pub fn concrete_eval(prog: &Program, inputs: &[i64]) -> Option<i64> {
    let mut stack: Vec<i64> = Vec::new();
    let mut locals = [0i64; 4];
    for v in inputs.iter() {
        stack.push(*v);
    }
    for op in &prog.ops {
        match op {
            OpKind::ConstI64(v) => stack.push(*v),
            OpKind::ConstBool(b) => stack.push(if *b { 1 } else { 0 }),
            OpKind::ConstStr(_) => stack.push(0),
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
            OpKind::AndBool => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(if a != 0 && b != 0 { 1 } else { 0 });
            }
            OpKind::OrBool => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                stack.push(if a != 0 || b != 0 { 1 } else { 0 });
            }
            OpKind::NotBool => {
                let a = stack.pop()?;
                stack.push(if a == 0 { 1 } else { 0 });
            }
            OpKind::LocalGet { slot, .. } => {
                let v = locals[*slot as usize % locals.len()];
                stack.push(v);
            }
            OpKind::LocalSet { slot, .. } => {
                let v = stack.pop()?;
                locals[*slot as usize % locals.len()] = v;
            }
            OpKind::Cast { from: _, to } => {
                let v = stack.pop()?;
                if to.0 == SUB_ID {
                    // The runtime cast TRAPS on out-of-range values.
                    if v < SUB_LO || v > SUB_HI {
                        return None; // trap
                    }
                }
                stack.push(v);
            }
            OpKind::Bitcast { .. } => {}
            OpKind::Load { .. } => {
                let _ = stack.pop()?;
                stack.push(0); // reads anything (the abstraction is ⊤)
            }
            OpKind::Store { .. } => {
                let _ = stack.pop()?;
                let _ = stack.pop()?;
            }
            // Ops the generator never emits (control flow, calls, mmio…):
            _ => return None, // treat as a trap / opaque — excluded by generator
        }
    }
    stack.last().copied()
}

// Abstract semantics ----------------------------------------------------------

/// The discharge verdict of a program: `(tri_of_obligation, final_interval)`.
/// For `cast_site` programs the obligation is on the PRE-cast value.
pub fn abstract_verdict(prog: &Program) -> (Tri, Interval) {
    let mut st = State::callee_entry(prog.n_inputs, 4);
    // Seed the abstract inputs from the domains.
    st.stack.clear();
    for &(lo, hi) in &prog.domains {
        st.stack.push(verifier::interp::Slot {
            iv: Interval::Range { lo, hi },
            origin: verifier::interp::Origin::Arg(0),
        });
    }
    let mut pre_cast: Option<Interval> = None;
    for op in &prog.ops {
        if matches!(op, OpKind::Cast { to, .. } if to.0 == SUB_ID) {
            pre_cast = Some(st.top_interval());
        }
        st.step(op, sr());
    }
    let (tri, val) = if prog.cast_site {
        let v = pre_cast.unwrap_or(Interval::TOP);
        (eval_in_range(v, prog.target.0, prog.target.1), v)
    } else {
        let v = st.top_interval();
        (eval_in_range(v, prog.target.0, prog.target.1), v)
    };
    (tri, val)
}

/// The soundness property: `Discharged ⇒ no witness violates`.
fn assert_discharge_sound(prog: &Program, tag: &str) {
    let (tri, _val) = abstract_verdict(prog);
    let mut inputs = Vec::new();
    enumerate_inputs(&prog.domains, &mut inputs);
    let (tlo, thi) = prog.target;
    match tri {
        Tri::DefTrue => {
            for input in &inputs {
                match concrete_eval(prog, input) {
                    None => panic!(
                        "{tag}: DISCHARGE applied but concrete run traps on {input:?} — unsound"
                    ),
                    Some(v) => assert!(
                        tlo <= v && v <= thi,
                        "{tag}: DISCHARGE but {input:?} evaluates to {v} outside [{tlo},{thi}] — unsound"
                    ),
                }
            }
        }
        Tri::DefFalse => {
            for input in &inputs {
                let v = concrete_eval(prog, input);
                match v {
                    Some(v) => assert!(
                        v < tlo || v > thi,
                        "{tag}: DefFalse but {input:?} evaluates to {v} INSIDE [{tlo},{thi}]"
                    ),
                    // A trap also "violates" the target in-range predicate.
                    None => {}
                }
            }
        }
        Tri::Top => {} // open: nothing to check; the check stays (conservative)
    }
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Stack-safe random straight-line program. `cast_final` produces a program
/// whose last op is a narrowing casts (the obligation sits on the cast pre).
fn gen_program(rng: &mut Rng, n_inputs: usize, cast_final: bool) -> Program {
    let domains: Vec<(i64, i64)> = (0..n_inputs)
        .map(|_| {
            let lo = rng.i64_between(-3, 0);
            let hi = rng.i64_between(1, 4);
            (lo, hi)
        })
        .collect();
    let ops: Vec<OpKind> = gen_op_chain(rng, n_inputs, cast_final);
    // A cast-site program's obligation IS the cast itself — its target is the
    // subtype range, not an arbitrary target (the cast traps on values
    // outside it).
    let target = if cast_final {
        (SUB_LO, SUB_HI)
    } else {
        let tlo = rng.i64_between(-6, 2);
        let thi = rng.i64_between(3, 10);
        (tlo, thi)
    };
    Program {
        n_inputs,
        domains,
        ops,
        target,
        cast_site: cast_final,
    }
}

fn gen_op_chain(rng: &mut Rng, n_inputs: usize, cast_final: bool) -> Vec<OpKind> {
    // The generator tracks a symbolic stack so it only emits valid pops
    // (the abstract/concrete evaluators treat underflow as a trap, so the
    // programs must be well-typed by construction).
    let mut depth = n_inputs.max(1);
    let mut ops: Vec<OpKind> = Vec::new();
    let length = 1 + rng.below(8) as usize; // 1..=8 ops
    let budget = if cast_final { length } else { length };
    for _ in 0..budget {
        // Avoid emitting the final cast before the last slot.
        if cast_final && ops.len() + 1 == budget {
            break;
        }
        let choice = rng.below(11);
        match choice {
            0 => {
                ops.push(OpKind::ConstI64(rng.i64_between(-5, 5)));
                depth += 1;
            }
            1 | 2 | 3 => {
                if depth < 2 {
                    // not enough operands — retry next time
                    continue;
                }
                ops.push(match choice {
                    1 => OpKind::AddI64,
                    2 => OpKind::SubI64,
                    _ => OpKind::MulI64,
                });
                depth -= 1;
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
                    ops.push(OpKind::Swap {
                        a: TypeId(0),
                        b: TypeId(0),
                    });
                }
            }
            7 => {
                if depth >= 1 {
                    ops.push(OpKind::LocalSet { slot: 0, ty: TypeId(0) });
                    depth -= 1;
                }
            }
            // LocalGet of slot 0 is a +1 net op; on the FIRST op it may read a
            // never-written local (⊤ abstract / 0 concrete) — sound either way.
            8 => {
                ops.push(OpKind::LocalGet { slot: 0, ty: TypeId(0) });
                depth += 1;
            }
            9 => {
                if depth >= 2 {
                    ops.push(OpKind::Cmp {
                        out: TypeId(1),
                        kind: CmpKind::Eq,
                    });
                    depth -= 1;
                }
            }
            _ => {
                ops.push(OpKind::Bitcast {
                    from: TypeId(0),
                    to: TypeId(0),
                });
            }
        }
    }
    if cast_final {
        ops.push(OpKind::Cast {
            from: TypeId(0),
            to: TypeId(SUB_ID),
        });
    }
    ops
}

// ---------------------------------------------------------------------------
// Suites
// ---------------------------------------------------------------------------

/// Crafted programs whose hand-audited shapes are known (Q9: "exhaustive
/// over tiny domains for crafted suites").
#[test]
fn crafted_suite_exhaustive_over_tiny_domains() {
    let mk = |domains: Vec<(i64, i64)>, ops: Vec<OpKind>, target: (i64, i64), cast: bool| Program {
        n_inputs: domains.len(),
        domains,
        ops,
        target,
        cast_site: cast,
    };

    // 100 - 50 in-range chain: `100 50 -` → [50,50] ⊆ [0,100].
    let p = mk(
        vec![],
        vec![OpKind::ConstI64(100), OpKind::ConstI64(50), OpKind::SubI64],
        (0, 100),
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_discharge_sound(&p, "100-50");

    // 150 out-of-range constant → provably failing.
    let p = mk(vec![], vec![OpKind::ConstI64(150)], (0, 100), false);
    assert_eq!(abstract_verdict(&p).0, Tri::DefFalse);
    assert_discharge_sound(&p, "150");

    // 3 input arithmetic: (a*b)+c against a broad target.
    let p = mk(
        vec![(-2, 2), (-2, 2), (-2, 2)],
        vec![OpKind::MulI64, OpKind::AddI64],
        (100, 200), // (a*b)+c never reaches here
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefFalse);
    assert_discharge_sound(&p, "a*b+c");

    // (a*b)+c in range for the whole tiny input cube.
    let p = mk(
        vec![(0, 1), (0, 1), (0, 1)],
        vec![OpKind::MulI64, OpKind::AddI64],
        (0, 100),
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_discharge_sound(&p, "a*b+c-in-range");

    // Two-argument subtraction with a mixed domain → Top (open).
    let p = mk(
        vec![(-3, 3), (-3, 3)],
        vec![OpKind::SubI64],
        (0, 3),
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::Top);
    assert_discharge_sound(&p, "mixed-sub");

    // Cast final: pre-cast interval wholly in range discharges the cast.
    let p = mk(
        vec![(0, 10)],
        vec![
            OpKind::ConstI64(0),
            OpKind::AddI64,
            OpKind::Cast { from: TypeId(0), to: TypeId(SUB_ID) },
        ],
        (0, 100),
        true,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_discharge_sound(&p, "cast-in-range");

    // Cast final: input domain above the target — the cast always traps.
    let p = mk(
        vec![(150, 200)],
        vec![OpKind::Cast { from: TypeId(0), to: TypeId(SUB_ID) }],
        (0, 100),
        true,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefFalse);
    assert_discharge_sound(&p, "cast-always-trap");

    // Cast final: input domain straddling the boundary → open.
    let p = mk(
        vec![(-20, 200)],
        vec![OpKind::Cast { from: TypeId(0), to: TypeId(SUB_ID) }],
        (0, 100),
        true,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::Top);
    assert_discharge_sound(&p, "cast-straddle");

    // Locals: repeated set/get preserve the value flow.
    let p = mk(
        vec![(0, 5)],
        vec![
            OpKind::LocalSet { slot: 1, ty: TypeId(0) },
            OpKind::LocalGet { slot: 1, ty: TypeId(0) },
        ],
        (0, 5),
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_discharge_sound(&p, "local-roundtrip");

    // Dup/Drop/Swap are no-ops on the value flow.
    let p = mk(
        vec![(0, 5)],
        vec![OpKind::Dup { ty: TypeId(0) }, OpKind::Drop { ty: TypeId(0) }],
        (0, 5),
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_discharge_sound(&p, "dup-drop");

    // Load cuts the value flow to ⊤ → open, never discharged.
    let p = mk(
        vec![(0, 5)],
        vec![OpKind::Load { ty: TypeId(0) }],
        (0, 5),
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::Top, "Load ⇒ ⊤ (Q4)");
}

/// ≥ 10^5 seeded random straight-line programs, every one exhaustively
/// checked against its concrete interpretation over its own tiny input
/// domains (NFR-3: zero false-Discharged).
#[test]
fn random_straight_line_soundness_100k() {
    let mut rng = Rng(0x0d15_a5e_0d15_a5e);
    let mut discharged = 0u64;
    let mut provably_failing = 0u64;
    let mut open = 0u64;
    const N: u64 = 100_000;
    let mut first_mismatch = None;
    for idx in 0..N {
        let n_inputs = 1 + (rng.below(3) as usize); // 1..=3
        let cast_final = rng.below(4) == 0; // ~25% cast-site programs
        let prog = gen_program(&mut rng, n_inputs, cast_final);
        let (tri, _) = abstract_verdict(&prog);
        match tri {
            Tri::DefTrue => {
                discharged += 1;
                if let Some(inputs) = find_violating(&prog) {
                    first_mismatch = Some((inputs, tri, idx));
                }
            }
            Tri::DefFalse => {
                provably_failing += 1;
                if let Some(inputs) = find_satisfying(&prog) {
                    first_mismatch = Some((inputs, tri, idx));
                }
            }
            Tri::Top => open += 1,
        }
        assert!(
            first_mismatch.is_none(),
            "first violation at program {idx}: {:?}\nprogram: {prog:?}",
            first_mismatch
        );
    }
    // Sanity that the generator exercised all three verdicts (a degenerate
    // generator would silently pass).
    assert!(discharged > 0, "generator produced no discharges");
    assert!(provably_failing > 0, "generator produced no provably-failing");
    assert!(open > 0, "generator produced no open verdicts");
}

fn find_violating(prog: &Program) -> Option<Vec<i64>> {
    let mut inputs = Vec::new();
    enumerate_inputs(&prog.domains, &mut inputs);
    let (tlo, thi) = prog.target;
    for input in inputs {
        let r = concrete_eval(prog, &input);
        let bad = match r {
            None => true, // a discharge side's cast must not trap
            Some(v) => !(tlo <= v && v <= thi),
        };
        if bad {
            return Some(input);
        }
    }
    None
}

fn find_satisfying(prog: &Program) -> Option<Vec<i64>> {
    let mut inputs = Vec::new();
    enumerate_inputs(&prog.domains, &mut inputs);
    let (tlo, thi) = prog.target;
    for input in inputs {
        if let Some(v) = concrete_eval(prog, &input) {
            if tlo <= v && v <= thi {
                return Some(input);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Loop CFG flavor: the back-edge widening (FR-12) must never discharge a
// loop-carried value (a false discharge would be an unsound elision).
// ---------------------------------------------------------------------------

/// Build the synthetic word:
/// `x = input; while (x < 10) { x = x + 1 }; ret x` over input n ∈ [0,5].
/// True concrete finals: 10.., so `target [0,8]` gives DefFalse SOUND ans,
/// but the abstract engine must widen x to ⊤ (open) — discharging would be
/// unsound (x=0 loops to 10 ∉ [0,8]).
fn loop_word() -> ir::Word {
    let mut w = ir::Word {
        name: Atom::new(b"loopish").unwrap(),
        sig: Sig {
            in_len: 1,
            out_len: 1,
            ..Sig::empty()
        },
        performs: ir::EffectSet::empty(),
        requires: ir::CapSet::empty(),
        bound: ir::StackBound::ID,
        entry: BlockId(0),
        types: Default::default(),
        type_sizes: Default::default(),
        type_classes: Default::default(),
        apertures: Default::default(),
        subtype_bases: Default::default(),
        blocks: Default::default(),
    };
    let ty = TypeId(0);
    let mut bx = |ops: Vec<OpKind>| -> BlockId {
        let id = BlockId(w.blocks.len() as u16);
        let mut b = ir::Block {
            id,
            entry_stack: Default::default(),
            ops: Default::default(),
        };
        for (i, op) in ops.into_iter().enumerate() {
            b.ops.push(ir::Op { kind: op, span: Span::UNKNOWN }).unwrap_or_else(|_| panic!("op {i}"));
        }
        w.blocks.push(b).expect("block fits");
        id
    };
    // block 0: x → local 1, br 1
    let _b0 = bx(vec![
        OpKind::LocalSet { slot: 1, ty },
        OpKind::Br { target: BlockId(1) },
    ]);
    // block 1 (header): if x < 10 → block 2 (body) else block 3 (exit)
    bx(vec![
        OpKind::LocalGet { slot: 1, ty },
        OpKind::ConstI64(10),
        OpKind::Cmp { out: TypeId(1), kind: CmpKind::Lt },
        OpKind::BrIf { then_tgt: BlockId(2), else_tgt: BlockId(3) },
    ]);
    // block 2 (body): x = x + 1; br 1
    bx(vec![
        OpKind::LocalGet { slot: 1, ty },
        OpKind::ConstI64(1),
        OpKind::AddI64,
        OpKind::LocalSet { slot: 1, ty },
        OpKind::Br { target: BlockId(1) },
    ]);
    // block 3 (exit): ret x
    bx(vec![
        OpKind::LocalGet { slot: 1, ty },
        OpKind::Ret,
    ]);
    w
}

/// Regression probe for the random-suite shape: `[Drop]` with 2 inputs and a
/// disjoint target must never claim a satisfying run.
#[test]
fn drop_disjoint_target_probe() {
    let p = Program {
        n_inputs: 2,
        domains: vec![(-2, 1), (0, 3)],
        ops: vec![OpKind::Drop { ty: TypeId(0) }],
        target: (2, 8),
        cast_site: false,
    };
    assert_eq!(abstract_verdict(&p).0, Tri::DefFalse);
    assert!(
        find_satisfying(&p).is_none(),
        "Drop of b leaves a ∈ [-2,1], never in (2,8)"
    );
}

#[test]
fn loop_back_edge_widening_never_false_discharges() {
    let w = loop_word();
    let cf = run_cfg(&w, sr());
    let exit = verifier::interp::exit_state(&cf, &w, sr());
    eprintln!(
        "exit stack: {:?}",
        exit.stack.iter().map(|s| s.iv).collect::<Vec<_>>()
    );
    let final_iv = exit.top_interval();
    assert_eq!(
        final_iv,
        Interval::TOP,
        "loop-carried x must widen to ⊤ at the back-edge (FR-12)"
    );
    // With the widened exit, target [0,8] is open — never discharged.
    let tri = eval_in_range(final_iv, 0, 8);
    assert_eq!(tri, Tri::Top, "widen must keep the obligation open");
    // Concrete ground truth: for inputs 0..=5 the loop exits with x ≥ 10,
    // all OUTSIDE [0,8] — prooving that a discharge would have been a false
    // discharge (this is the whole point of widening).
    for n in 0..=5i64 {
        let mut x = n;
        while x < 10 {
            x += 1;
        }
        assert!(x > 8, "concrete ground truth: x={x} outside [0,8]");
    }
}