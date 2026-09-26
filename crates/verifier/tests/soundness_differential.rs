//! Differential soundness harness for the interval engine (static-
//! verification.md Q9/P5, FR-20/NFR-3; PLAN-VERIFY-3 P2).
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
//! random straight-line programs per run. The generator lives in
//! `verifier::gen` (PLAN-VERIFY-3 P2.3 — one generator, shared with future
//! port differentials); the concrete evaluator stays here (it is the *test's*
//! ground truth, not a shared surface).
//!
//! P2 adds the **per-target** suite: the reference semantics is a function of
//! `(TargetSpec, MemModel)` (§Q3), and since the IR data ops are full-width
//! i64 on every target (the runtime emulates 64-bit arithmetic on 32-bit
//! targets), the discharge behavior MUST be identical across all four
//! recognized targets under the default `FlatMem` model. The suite asserts
//! per-program identity (tri + final interval) across targets — the width
//! parameterization must be a *no-op on the data domain*.

use ir::{Atom, BlockId, CmpKind, OpKind, Sig, Span, TypeId};
use std::collections::HashMap;
use verifier::gen::{gen_program, MemDesc, Program, Rng, SUB_HI, SUB_ID, SUB_LO};
use verifier::interp::{
    exit_state, run_cfg, ApertureMem, FlatMem, Origin, Slot, State, SubtypeRange,
};
use verifier::interval::{eval_in_range, Interval, Tri};
use verifier::target::TargetSpec;

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

// -------- Program model & enumeration ----------------------------------------

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
/// rejected an out-of-range value — the runtime trap). Memory mirrors the
/// abstract [`ApertureMem`] (PLAN-VERIFY-3 P2): point addresses within the
/// program's RAM region record stores and replay them on load; a never-
/// written cell reads as 0 ("any value"); MMIO reads answer the program's
/// scripted register value (0 when unscripted).
pub fn concrete_eval(prog: &Program, inputs: &[i64]) -> Option<i64> {
    let mut stack: Vec<i64> = Vec::new();
    let mut locals = [0i64; 4];
    let mut ram: HashMap<i64, i64> = HashMap::new();
    let (ram_lo, ram_hi) = match prog.mem.ram {
        Some((lo, hi)) => (lo as i128, hi as i128),
        None => (0, 0),
    };
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
                    if !(SUB_LO..=SUB_HI).contains(&v) {
                        return None; // trap
                    }
                }
                stack.push(v);
            }
            OpKind::Bitcast { .. } => {}
            OpKind::Load { .. } => {
                let addr = stack.pop()?;
                // Reads are RAM-modeled (P2): a point address inside the
                // program's region replays the recorded store; anything else
                // reads "any value" (0 here — the abstract answer is ⊤).
                let inside = (addr as i128) >= ram_lo && (addr as i128) <= ram_hi;
                let v = if inside {
                    ram.get(&addr).copied().unwrap_or(0)
                } else {
                    0
                };
                stack.push(v);
            }
            OpKind::Store { .. } => {
                // `[addr, value]` — value is on top; the runtime pops value,
                // then address.
                let val = stack.pop()?;
                let addr = stack.pop()?;
                let inside = (addr as i128) >= ram_lo && (addr as i128) <= ram_hi;
                if inside {
                    ram.insert(addr, val);
                }
            }
            OpKind::MmioVolLoad { place, .. } | OpKind::MmioVolLoadField { place, .. } => {
                let _ = stack.pop()?;
                let scripted = prog
                    .mem
                    .scripted
                    .iter()
                    .find(|(k, _)| k == place)
                    .map(|(_, v)| *v)
                    .unwrap_or(0);
                stack.push(scripted);
            }
            OpKind::MmioVolStore { .. } | OpKind::MmioVolStoreField { .. } => {
                let _ = stack.pop()?;
                let _ = stack.pop()?;
            }
            // Ops the generator never emits (control flow, calls, …):
            _ => return None, // treat as a trap / opaque — excluded by generator
        }
    }
    stack.last().copied()
}

// Abstract semantics ----------------------------------------------------------

/// The discharge verdict of a program under a given target identity:
/// `(tri_of_obligation, final_interval)`. For `cast_site` programs the
/// obligation is on the PRE-cast value. The memory model is the program's
/// [`MemDesc`]: `FlatMem` when no surface is enabled, else an `ApertureMem`
/// carrying the program's RAM region and scripted MMIO reads (P2 — the
/// differential steps the actual `(TargetSpec, MemModel)` pair, §Q3).
pub fn abstract_verdict(prog: &Program) -> (Tri, Interval) {
    let mut flat = FlatMem;
    let mut ap: Option<ApertureMem> = None;
    if prog.mem.ram.is_some() || !prog.mem.scripted.is_empty() {
        let mut m = ApertureMem::new(prog.mem.ram.unwrap_or((0, 0)));
        for &(k, v) in prog.mem.scripted.iter() {
            m.script_read(&k, Interval::const_val(v));
        }
        ap = Some(m);
    }
    let mut st = State::callee_entry(prog.n_inputs(), 4);
    // Seed the abstract inputs from the domains.
    st.stack.clear();
    for &(lo, hi) in &prog.domains {
        st.stack.push(Slot {
            iv: Interval::Range { lo, hi },
            origin: Origin::Arg(0),
        });
    }
    let mut pre_cast: Option<Interval> = None;
    for op in &prog.ops {
        if matches!(op, OpKind::Cast { to, .. } if to.0 == SUB_ID) {
            pre_cast = Some(st.top_interval());
        }
        match &mut ap {
            Some(m) => st.step(op, sr(), prog.spec, m),
            None => st.step(op, sr(), prog.spec, &mut flat),
        }
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
                // A trap also "violates" the target in-range predicate (the
                // cast's check stops control flow before the return).
                if let Some(v) = concrete_eval(prog, input) {
                    assert!(
                        v < tlo || v > thi,
                        "{tag}: DefFalse but {input:?} evaluates to {v} INSIDE [{tlo},{thi}]"
                    );
                }
            }
        }
        Tri::Top => {} // open: nothing to check; the check stays (conservative)
    }
}

// ---------------------------------------------------------------------------
// Suites
// ---------------------------------------------------------------------------

/// Crafted programs whose hand-audited shapes are known (Q9: "exhaustive
/// over tiny domains for crafted suites").
#[test]
fn crafted_suite_exhaustive_over_tiny_domains() {
    let mk = |domains: Vec<(i64, i64)>, ops: Vec<OpKind>, target: (i64, i64), cast: bool| Program {
        domains,
        ops,
        target,
        cast_site: cast,
        spec: TargetSpec::X86_64,
        mem: MemDesc::flat(),
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
    let p = mk(vec![(-3, 3), (-3, 3)], vec![OpKind::SubI64], (0, 3), false);
    assert_eq!(abstract_verdict(&p).0, Tri::Top);
    assert_discharge_sound(&p, "mixed-sub");

    // Cast final: pre-cast interval wholly in range discharges the cast.
    let p = mk(
        vec![(0, 10)],
        vec![
            OpKind::ConstI64(0),
            OpKind::AddI64,
            OpKind::Cast {
                from: TypeId(0),
                to: TypeId(SUB_ID),
            },
        ],
        (0, 100),
        true,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_discharge_sound(&p, "cast-in-range");

    // Cast final: input domain above the target — the cast always traps.
    let p = mk(
        vec![(150, 200)],
        vec![OpKind::Cast {
            from: TypeId(0),
            to: TypeId(SUB_ID),
        }],
        (0, 100),
        true,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefFalse);
    assert_discharge_sound(&p, "cast-always-trap");

    // Cast final: input domain straddling the boundary → open.
    let p = mk(
        vec![(-20, 200)],
        vec![OpKind::Cast {
            from: TypeId(0),
            to: TypeId(SUB_ID),
        }],
        (0, 100),
        true,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::Top);
    assert_discharge_sound(&p, "cast-straddle");

    // Locals: repeated set/get preserve the value flow.
    let p = mk(
        vec![(0, 5)],
        vec![
            OpKind::LocalSet {
                slot: 1,
                ty: TypeId(0),
            },
            OpKind::LocalGet {
                slot: 1,
                ty: TypeId(0),
            },
        ],
        (0, 5),
        false,
    );
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_discharge_sound(&p, "local-roundtrip");

    // Dup/Drop/Swap are no-ops on the value flow.
    let p = mk(
        vec![(0, 5)],
        vec![
            OpKind::Dup { ty: TypeId(0) },
            OpKind::Drop { ty: TypeId(0) },
        ],
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
    let mut rng = Rng::new(0x0d15_a5e0_d15a_5e0d);
    let mut discharged = 0u64;
    let mut provably_failing = 0u64;
    let mut open = 0u64;
    const N: u64 = 100_000;
    let mut first_mismatch = None;
    for idx in 0..N {
        let n_inputs = 1 + (rng.below(3) as usize); // 1..=3
        let cast_final = rng.below(4) == 0; // ~25% cast-site programs
        let prog = gen_program(
            &mut rng,
            TargetSpec::X86_64,
            &MemDesc::flat(),
            n_inputs,
            cast_final,
        );
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
    assert!(
        provably_failing > 0,
        "generator produced no provably-failing"
    );
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
// Per-target width-relativism (PLAN-VERIFY-3 §Q3, P2): the discharge verdict
// of a data-only program MUST be identical under every recognized target
// (the runtime emulates 64-bit data arithmetic on 32-bit targets; only the
// address/usize and MMIO domains are width-relative, and FlatMem never
// touches them). A second generator run at an independent seed keeps the
// sample fresh per CI run while remaining deterministic.
// ---------------------------------------------------------------------------

const PER_TARGET_SAMPLE: u64 = 5_000;

#[test]
fn per_target_identity_of_data_domain_discharge() {
    let targets = [
        TargetSpec::X86_64_UNKNOWN_LINUX_GNU,
        TargetSpec::X86_64_UNKNOWN_NONE,
        TargetSpec::ARM_V7M_UNKNOWN_NONE,
        TargetSpec::RISCV32_UNKNOWN_NONE,
    ];
    // The reference spec is x86_64-unknown-none (the in-tree default).
    let reference = TargetSpec::X86_64;
    for seed in 0..4u64 {
        let mut rng = Rng::new(0x5eed_00d5_0000_0000 + seed);
        for idx in 0..PER_TARGET_SAMPLE {
            let n_inputs = 1 + (rng.below(3) as usize);
            let cast_final = rng.below(4) == 0;
            let base = gen_program(&mut rng, reference, &MemDesc::flat(), n_inputs, cast_final);
            let (ref_tri, ref_iv) = abstract_verdict(&base);
            // Same program, every target: verdicts must be identical.
            for &spec in targets.iter() {
                if spec == reference {
                    continue;
                }
                let mut p = base.clone();
                p.spec = spec;
                let (tri, iv) = abstract_verdict(&p);
                assert_eq!(
                    (tri, iv),
                    (ref_tri, ref_iv),
                    "seed {seed} idx {idx} ({spec:?}): data-domain discharge differs from {} — \
                     the width parameterization changed semantics",
                    reference.triple
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Memory-model boundary differential (PLAN-VERIFY-3 P2.1/P2.3): programs that
// carry a [`MemDesc`] — point `Store`/`Load` pairs inside a modeled RAM
// region and scripted MMIO reads — are checked against the concrete RAM the
// same way the data programs are checked against wrapping-i64 execution. The
// abstract engine answers a store-then-load point with the stored interval
// and a scripted MMIO read with the scripted value; a false Discharged would
// mean the `(TargetSpec, MemModel)` parametrization elided a check the
// concrete memory would violate.
// ---------------------------------------------------------------------------

fn memory_suite_mem() -> MemDesc {
    MemDesc::ram_and_dev_script(0x1000, 0x2000, 7)
}

/// ≥ 10^5 memory-bearing programs, exhaustively checked against the concrete
/// RAM (zero false-Discharged). Also conservatively exercises the
/// unscripted-MMIO path (reads answer 0 concretely, `⊤` abstractly — open).
#[test]
fn memory_differential_soundness_100k() {
    let mem = memory_suite_mem();
    let mut rng = Rng::new(0xdead_beef_d00d_cafe);
    let mut op_counter = [0u64; 4]; // [store, load, mmio-read, mmio-write]
    const N: u64 = 75_000;
    let mut first_mismatch: Option<String> = None;
    for idx in 0..N {
        let n_inputs = 1 + (rng.below(3) as usize);
        let cast_final = rng.below(4) == 0;
        let prog = gen_program(&mut rng, TargetSpec::X86_64, &mem, n_inputs, cast_final);
        for op in &prog.ops {
            match op {
                OpKind::Store { .. } => op_counter[0] += 1,
                OpKind::Load { .. } => op_counter[1] += 1,
                OpKind::MmioVolLoad { .. } | OpKind::MmioVolLoadField { .. } => op_counter[2] += 1,
                OpKind::MmioVolStore { .. } | OpKind::MmioVolStoreField { .. } => {
                    op_counter[3] += 1
                }
                _ => {}
            }
        }
        let (tri, _) = abstract_verdict(&prog);
        if tri == Tri::DefTrue {
            if let Some(inputs) = find_violating(&prog) {
                first_mismatch = Some(format!(
                    "memory idx {idx}: DISCHARGE but {inputs:?} violates ({prog:?})"
                ));
            }
        } else if tri == Tri::DefFalse {
            if let Some(inputs) = find_satisfying(&prog) {
                first_mismatch = Some(format!(
                    "memory idx {idx}: DefFalse but {inputs:?} satisfies ({prog:?})"
                ));
            }
        }
        assert!(
            first_mismatch.is_none(),
            "{}",
            first_mismatch.as_deref().unwrap_or_default()
        );
    }
    // The generator really exercised the memory surface (a degenerate
    // descriptor would silently pass).
    assert!(op_counter[0] > 0, "no Store emitted in the memory suite");
    assert!(op_counter[1] > 0, "no Load emitted in the memory suite");
    assert!(
        op_counter[2] > 0,
        "no MMIO read emitted in the memory suite"
    );
    eprintln!(
        "memory suite op mix: store={} load={} mmio-read={} mmio-write={}",
        op_counter[0], op_counter[1], op_counter[2], op_counter[3]
    );
}

/// The memory suite is *specific*: a store→load pair at a point address
/// discharges only when the stored value is in range (abstract [v,v] matching
/// concrete v), and a scripted MMIO read discharges exactly the scripted
/// value. Both directions are proven concrete-sound.
#[test]
fn memory_store_load_pair_is_sound_in_both_directions() {
    let mem = memory_suite_mem();
    let addr = 0x1234i64;
    let mk_pair = |value: i64, target: (i64, i64)| Program {
        domains: vec![],
        ops: vec![
            OpKind::ConstI64(addr),
            OpKind::ConstI64(value),
            OpKind::Store { ty: TypeId(0) },
            OpKind::ConstI64(addr),
            OpKind::Load { ty: TypeId(0) },
        ],
        target,
        cast_site: false,
        spec: TargetSpec::X86_64,
        mem: mem.clone(),
    };
    // Stored 7 ∈ [0,100] → the load discharges; the concrete load returns 7.
    let p = mk_pair(7, (0, 100));
    assert_eq!(abstract_verdict(&p).0, Tri::DefTrue);
    assert_eq!(concrete_eval(&p, &[]), Some(7));
    // Stored 150 ∉ [0,100] → provably failing; the concrete load returns 150.
    let p = mk_pair(150, (0, 100));
    assert_eq!(abstract_verdict(&p).0, Tri::DefFalse);
    assert_eq!(concrete_eval(&p, &[]), Some(150));

    // A scripted MMIO read answers the script: 7 → discharge, and the
    // concrete read mirrors it.
    let dev = Atom::new(b"dev").unwrap();
    let read_prog = |target: (i64, i64)| Program {
        domains: vec![],
        ops: vec![
            OpKind::ConstI64(0),
            OpKind::MmioVolLoad {
                ty: TypeId(0),
                place: dev,
                read_kind: ir::ReadKind::Plain,
                atomic_max: 64,
                barrier: ir::BarrierKind::None,
            },
        ],
        target,
        cast_site: false,
        spec: TargetSpec::X86_64,
        mem: mem.clone(),
    };
    assert_eq!(abstract_verdict(&read_prog((0, 100))).0, Tri::DefTrue);
    assert_eq!(concrete_eval(&read_prog((0, 100)), &[]), Some(7));
}

// ---------------------------------------------------------------------------
// Loop CFG flavor: the back-edge widening (FR-12) must never discharge a
// loop-carried value (a false discharge would be an unsound elision).
// ---------------------------------------------------------------------------

fn loop_word() -> ir::Word {
    let mut w = ir::Word {
        name: ir::Atom::new(b"loopish").unwrap(),
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
            b.ops
                .push(ir::Op {
                    kind: op,
                    span: Span::UNKNOWN,
                })
                .unwrap_or_else(|_| panic!("op {i}"));
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
        OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Lt,
        },
        OpKind::BrIf {
            then_tgt: BlockId(2),
            else_tgt: BlockId(3),
        },
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
    bx(vec![OpKind::LocalGet { slot: 1, ty }, OpKind::Ret]);
    w
}

/// Regression probe for the random-suite shape: `[Drop]` with 2 inputs and a
/// disjoint target must never claim a satisfying run.
#[test]
fn drop_disjoint_target_probe() {
    let p = Program {
        domains: vec![(-2, 1), (0, 3)],
        ops: vec![OpKind::Drop { ty: TypeId(0) }],
        target: (2, 8),
        cast_site: false,
        spec: TargetSpec::X86_64,
        mem: MemDesc::flat(),
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
    let mut mem = FlatMem;
    let cf = run_cfg(&w, sr(), TargetSpec::X86_64, &mut mem);
    let exit = exit_state(&cf, &w, sr(), TargetSpec::X86_64, &mut mem);
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
