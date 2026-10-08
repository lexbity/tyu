//! Property tests over the production engine's transaction surface
//! (PLAN-WASM-1 P2 §Q3/§7.3, R-3; proptest per §15). In-crate so the private
//! checkpoint/rollback machinery is exercised directly — the integration
//! props (G53) run over the fragment fork and the fixtures; these run over
//! the *production* engine and the real session.
//!
//! Two properties:
//!
//! 1. **Heap-rollback completeness** — an arbitrary sequence of region-ABI
//!    effects (create/alloc/store/reset/destroy), journaled exactly the way
//!    the engine journals them, is completely inverted by [`rollback`]: the
//!    heap is identical to its pre-phrase state, the journal is empty, and
//!    re-applying the same sequence on the rolled-back heap reproduces the
//!    same post-state (no residue a later phrase could observe). The shadow
//!    model is an independent handle/pointer recorder, not a second heap.
//!
//! 2. **Session transactional invariant** — arbitrary generated phrases
//!    (i64 literals, `dup`/`drop`/`swap`, `+ - *`, and a trap trigger) run
//!    through the real [`ReplSession`]: an `Ok` commits exactly the model's
//!    stack; a trap rolls back to the pre-phrase stack; a fuel exhaustion
//!    rolls back AND suspends, and `step` resumes to the same observable
//!    outcome as the fully-fueled run (§Q6/§Q8).

use alloc::vec::Vec;
use proptest::prelude::*;

use crate::engine::ExecState;
use crate::fuel::Fuel;
use crate::heap::RegionHeap;
use crate::journal::{Effect, Journal};
use crate::session::{rollback, Checkpoint, ReplSession, SessionOpts, Status};

// ---------------------------------------------------------------------------
// Property 1: heap-rollback completeness over generated effect sequences
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Act {
    Create(u64),
    Alloc(usize, u64),
    Store(usize, u64, i64),
    Reset(usize),
    Destroy(usize),
}

fn acts_strategy() -> impl Strategy<Value = Vec<Act>> {
    prop::collection::vec(
        prop_oneof![
            (8u64..=64).prop_map(Act::Create),
            (0usize..4usize, 8u64..=64).prop_map(|(i, sz)| Act::Alloc(i, sz)),
            (0usize..4usize, 0usize..4usize, any::<i64>())
                .prop_map(|(i, k, v)| Act::Store(i, k as u64, v)),
            (0usize..4usize).prop_map(Act::Reset),
            (0usize..4usize).prop_map(Act::Destroy),
        ],
        1..48,
    )
}

/// The independent shadow recorder: per handle, its id, liveness, and the
/// pointers `alloc` returned since the last reset (valid store targets — a
/// store below the cursor would be OutOfBounds, the heap's program-level
/// fault surface, not a rollback concern). The model never reads the heap's
/// internals.
#[derive(Default)]
struct Shadow {
    regions: Vec<HandleShadow>,
}

#[derive(Default)]
struct HandleShadow {
    handle: u64,
    alive: bool,
    valid_ptrs: Vec<u64>,
}

/// Apply one action exactly the way the engine does: mutate the heap and
/// journal the matching `Effect`. Returns false when the action is invalid
/// against the current heap (generator imprecision) — deterministic for a
/// given heap state, so skipped actions skip identically on re-apply.
fn apply(act: &Act, heap: &mut RegionHeap, journal: &mut Journal, shadow: &mut Shadow) -> bool {
    match *act {
        Act::Create(sz) => match heap.region_create(sz) {
            Ok((handle, old_cursor)) => {
                journal.push(Effect::RegionCreate { handle, old_cursor });
                shadow.regions.push(HandleShadow {
                    handle,
                    alive: true,
                    valid_ptrs: Vec::new(),
                });
                true
            }
            Err(_) => false,
        },
        Act::Alloc(i, sz) => {
            let Some(h) = shadow.regions.get_mut(i) else {
                return false;
            };
            if !h.alive {
                return false;
            }
            match heap.region_alloc(h.handle, sz) {
                Ok((ptr, old_off)) => {
                    journal.push(Effect::RegionAlloc {
                        handle: h.handle,
                        old_off: old_off as u32,
                    });
                    h.valid_ptrs.push(ptr);
                    true
                }
                Err(_) => false,
            }
        }
        Act::Store(i, k, v) => {
            let Some(h) = shadow.regions.get(i) else {
                return false;
            };
            if !h.alive || h.valid_ptrs.is_empty() {
                return false;
            }
            let addr = h.valid_ptrs[k as usize % h.valid_ptrs.len()];
            match heap.store(addr, 8, v as u64) {
                Ok(old) => {
                    journal.push(Effect::Store {
                        addr,
                        width: 8,
                        old,
                    });
                    true
                }
                Err(_) => false,
            }
        }
        Act::Reset(i) => {
            let Some(h) = shadow.regions.get_mut(i) else {
                return false;
            };
            if !h.alive {
                return false;
            }
            match heap.region_reset(h.handle) {
                Ok(old_off) => {
                    journal.push(Effect::RegionReset {
                        handle: h.handle,
                        old_off: old_off as u32,
                    });
                    h.valid_ptrs.clear();
                    true
                }
                Err(_) => false,
            }
        }
        Act::Destroy(i) => {
            let Some(h) = shadow.regions.get_mut(i) else {
                return false;
            };
            if !h.alive {
                return false;
            }
            match heap.region_destroy(h.handle) {
                Ok(saved) => {
                    journal.push(Effect::RegionDestroy {
                        handle: h.handle,
                        saved,
                    });
                    h.alive = false;
                    h.valid_ptrs.clear();
                    true
                }
                Err(_) => false,
            }
        }
    }
}

/// Apply the whole sequence (the per-run Create/handle bookkeeping is inside
/// `apply`), returning the applied-effect count.
fn apply_all(acts: &[Act], state: &mut ExecState, shadow: &mut Shadow) -> usize {
    let before = state.journal.iter().count();
    for a in acts {
        let _ = apply(a, &mut state.heap, &mut state.journal, shadow);
    }
    state.journal.iter().count() - before
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn rollback_completeness_over_generated_effect_sequences(acts in acts_strategy()) {
        let mut state =
            ExecState::in_session(
            Vec::new(),
            RegionHeap::new(),
            Fuel::new(0),
            false,
            Journal::new(),
            Vec::new(),
        );
        let snap = Checkpoint {
            stack: Vec::new(),
            fuel: state.fuel,
        };
        let pre = state.heap.clone();

        let mut shadow = Shadow::default();
        let applied = apply_all(&acts, &mut state, &mut shadow);
        let post = state.heap.clone();

        rollback(&mut state, &snap);
        let rolled_back = state.heap.clone();
        prop_assert_eq!(rolled_back, pre, "rollback must restore the exact pre-phrase heap");
        prop_assert_eq!(state.journal.iter().count(), 0, "rollback must clear the journal");

        // No residue: re-applying the same sequence on the rolled-back heap
        // reproduces the same post-state — the second run observes the
        // identical starting world, handles included (deterministic ids).
        let mut shadow2 = Shadow::default();
        let applied2 = apply_all(&acts, &mut state, &mut shadow2);
        prop_assert_eq!(applied2, applied, "skip behavior must be replay-deterministic");
        let re_applied = state.heap.clone();
        prop_assert_eq!(re_applied, post, "re-apply after rollback diverges — rollback left residue");
    }
}

// ---------------------------------------------------------------------------
// Property 2: the session transactional invariant over generated phrases
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Tok {
    Lit(i64),
    Dup,
    Drop,
    Swap,
    Add,
    Sub,
    Mul,
    Boom,
}

fn toks_strategy() -> impl Strategy<Value = Vec<Tok>> {
    prop::collection::vec(
        prop_oneof![
            (-1000i64..=1000).prop_map(Tok::Lit),
            Just(Tok::Dup),
            Just(Tok::Drop),
            Just(Tok::Swap),
            Just(Tok::Add),
            Just(Tok::Sub),
            Just(Tok::Mul),
            Just(Tok::Boom),
        ],
        1..18,
    )
}

/// Render the tokens to phrase text, dropping tokens that would underflow
/// the phrase-local model stack (so the generated program stays typable and
/// the model never needs ambient-stack knowledge).
fn render(toks: &[Tok]) -> (alloc::string::String, Vec<Tok>) {
    let mut depth: i64 = 0;
    let mut kept = Vec::new();
    for t in toks {
        let delta = match t {
            Tok::Boom => Some(0),
            Tok::Lit(_) => Some(1),
            Tok::Dup => {
                if depth > 0 {
                    Some(1)
                } else {
                    None
                }
            }
            Tok::Drop => {
                if depth > 0 {
                    Some(-1)
                } else {
                    None
                }
            }
            Tok::Swap => {
                if depth >= 2 {
                    Some(0)
                } else {
                    None
                }
            }
            Tok::Add | Tok::Sub | Tok::Mul => {
                if depth >= 2 {
                    Some(-1)
                } else {
                    None
                }
            }
        };
        if let Some(d) = delta {
            depth += d;
            kept.push(t.clone());
        }
    }
    let text = kept
        .iter()
        .map(|t| match t {
            Tok::Lit(n) => alloc::format!("{n}"),
            Tok::Dup => "dup".into(),
            Tok::Drop => "drop".into(),
            Tok::Swap => "swap".into(),
            Tok::Add => "+".into(),
            Tok::Sub => "-".into(),
            Tok::Mul => "*".into(),
            Tok::Boom => "trap-now".into(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    (text, kept)
}

/// The reference model: run the kept tokens over an i64 stack (wrapping
/// arithmetic — the engine uses explicit `wrapping_*`, §7.1). Returns the
/// final stack and whether the program trapped (hit the boom trigger).
fn model_run(toks: &[Tok], mut stack: Vec<i64>) -> (Vec<i64>, bool) {
    let mut trapped = false;
    'outer: for t in toks {
        match *t {
            Tok::Lit(n) => stack.push(n),
            Tok::Dup => {
                let v = *stack.last().expect("render guaranteed depth");
                stack.push(v);
            }
            Tok::Drop => {
                stack.pop();
            }
            Tok::Swap => {
                let n = stack.len();
                stack.swap(n - 1, n - 2);
            }
            Tok::Add => {
                let b = stack.pop().expect("depth");
                let a = stack.pop().expect("depth");
                stack.push(a.wrapping_add(b));
            }
            Tok::Sub => {
                let b = stack.pop().expect("depth");
                let a = stack.pop().expect("depth");
                stack.push(a.wrapping_sub(b));
            }
            Tok::Mul => {
                let b = stack.pop().expect("depth");
                let a = stack.pop().expect("depth");
                stack.push(a.wrapping_mul(b));
            }
            Tok::Boom => {
                trapped = true;
                break 'outer;
            }
        }
    }
    (stack, trapped)
}

/// The session's persistent stack, read through the public UI surface and
/// parsed back to i64s (bottom-first).
fn stack_cells_of(session: &ReplSession) -> Vec<i64> {
    session
        .stack_cells()
        .iter()
        .map(|c| {
            let hex = core::str::from_utf8(&c.value_hex).expect("hex is ascii");
            u64::from_str_radix(&hex[2..], 16).expect("cell hex parses") as i64
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn session_transaction_rolls_back_and_resumes(toks in toks_strategy()) {
        let (text, kept) = render(&toks);
        prop_assume!(!kept.is_empty());
        let (model_stack, booms) = model_run(&kept, Vec::new());

        let mut session = ReplSession::new(SessionOpts::default());
        let sub = |s: &mut ReplSession, src: &str| {
            s.transactional_submit(src.as_bytes(), &[])
        };
        prop_assert_eq!(sub(&mut session, ": trap-now ( -- ) recurse ; : recurse ( -- ) recurse ;").status, Status::Ok);

        // -- fully fueled run: Ok, or trap 10 when the program booms. -------
        let r = sub(&mut session, &text);
        if booms {
            let trapped10 = matches!(r.status, Status::Trapped { code: ir::TrapCode::StackOverflow, .. });
            prop_assert!(trapped10, "engine outcome must match the model's trap");
            prop_assert_eq!(stack_cells_of(&session), Vec::<i64>::new(), "a trap rolls the stack back");
        } else {
            prop_assert_eq!(r.status, Status::Ok, "engine outcome must match the model's Ok");
            prop_assert_eq!(stack_cells_of(&session), model_stack.clone(), "Ok commits exactly the model stack");
        }
        session.reset();

        // -- under-fueled run: suspension, rollback, and resume. -----------
        // (The reset above discarded the dictionary — re-establish it; the
        // definitions commit is deterministic, DEC-3.)
        prop_assert_eq!(sub(&mut session, ": trap-now ( -- ) recurse ; : recurse ( -- ) recurse ;").status, Status::Ok);
        session.set_fuel(23);
        let pre = stack_cells_of(&session);
        let r = sub(&mut session, &text);
        match r.status {
            Status::Ok => {
                // The phrase fit in the budget: the commit matches the model.
                prop_assert!(!booms);
                prop_assert_eq!(stack_cells_of(&session), model_stack);
            }
            Status::FuelExhausted { diag } => {
                prop_assert_eq!(diag.code, crate::diag::code::E9150_FUEL);
                prop_assert_eq!(stack_cells_of(&session), pre.clone(), "fuel exhaustion rolls the stack back");
                // Resume with a fresh budget: the resumed run reaches the
                // same observable outcome as the fully-fueled run.
                let r2 = session.step(10_000_000);
                if booms {
                    let resumed_trap = matches!(r2.status, Status::Trapped { .. });
                    prop_assert!(resumed_trap, "a resumed boom traps");
                    prop_assert_eq!(stack_cells_of(&session), pre.clone(), "a resumed trap rolls back too");
                } else {
                    prop_assert_eq!(r2.status, Status::Ok);
                    prop_assert_eq!(stack_cells_of(&session), model_stack.clone(), "resumed Ok commits the model stack");
                }
                // A settled suspension is gone: stepping again is E9155.
                let r3 = session.step(1);
                let e9155 = matches!(&r3.status, Status::CompileError { diag }
                    if diag.code == crate::diag::code::E9155_PHRASE);
                prop_assert!(e9155, "stepping a settled suspension is E9155");
            }
            other => prop_assert!(false, "unexpected status {other:?}"),
        }
    }
}
