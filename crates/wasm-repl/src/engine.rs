//! The concrete step engine (PLAN-WASM-1 §7.1): executes a verified
//! `ir::Module` (or one exported word of it) with the concrete dynamics of
//! `ir-op-semantics.md` §2/§3 at a `TargetSpec` (I-1 / FR-6).
//!
//! Design rules implemented here:
//!
//! - **Exhaustive `OpKind` match, no wildcard arm** (FR-4): adding an IR op
//!   without an interpreter arm fails the build — the correct failure
//!   direction.
//! - **Wrapping two's-complement arithmetic** (§3): explicit `wrapping_*`
//!   everywhere, matching native.
//! - **Reference-shaped aggregates** (§Q11, DEC-15): stack slots hold
//!   addresses; `dup` copies the reference. The aliasing probes (G53) pin
//!   this against native.
//! - **Deterministic** (§Q6): no clocks, RNGs, or map ordering in the engine.
//!   The `Peripheral` boundary is the sole nondeterminism source (P4).
//! - **L2/L3 enforcement** (§Q4): Call depth/type checks, `StackBound` depth
//!   checks, and `Cast`/`Bitcast` subtype-range checks are never elidable.

use alloc::vec;
use alloc::vec::Vec;
use ir::{BlockId, OpKind, Span, TrapCode};

use crate::builtins::{Builtin, BuiltinSignal};
use crate::compile::{Compiled, DictTarget};
use crate::diag::code;
use crate::fuel::{Fuel, CALL_DEPTH_CAP, STACK_CAP};
use crate::heap::RegionHeap;
use crate::journal::{Effect, Journal};
use crate::mmio::{apply_write_kind, extract_field, insert_field, width_mask};
use crate::periph::Peripheral;
use crate::session::StackCell;
use crate::value::{Slot, ValKind, Value};

// ---------------------------------------------------------------------------
// G53 red-path instrument (PLAN-WASM-1 §Q10 / AC-9)
// ---------------------------------------------------------------------------

/// Under `wasm-selftest`, `add_i64` executes as `sub_i64` so the execution
/// differential (tests/exec_parity.rs) MUST fail. Every default build is
/// uncorrupted (the byte-differential pins that).
#[cfg(feature = "wasm-selftest")]
fn selftest_corrupt_add() -> bool {
    true
}

/// See the feature-gated twin above (default build: no corruption).
#[cfg(not(feature = "wasm-selftest"))]
fn selftest_corrupt_add() -> bool {
    false
}

/// The outcome of a module/word run.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The run reached a top-level `ret`: `out` holds the diagnostic-channel
    /// bytes (the `testio` surface).
    Ok { out: Vec<u8> },
    /// A trap with the frozen numeric `TrapCode` + the faulting op's span.
    /// Output already emitted stays emitted (§Q3 DEC-6). Contract failures
    /// (trap 20, L2a/L2c and the lowered `needs` guards) carry the §Q6/DEC-22
    /// `TrapDetail` — which word declared what, and what the stack held.
    Trap {
        code: TrapCode,
        span: Span,
        detail: Option<TrapDetail>,
        out: Vec<u8>,
    },
    /// `testio.exit ( i64 -- )` — the module run terminated with a code
    /// (`0` = pass); the session survives.
    Exited { code: i64, out: Vec<u8> },
    /// Fuel exhausted (E9150 surface): deterministic, resumable. `span` is
    /// the op that would have executed next (the E9150 diagnostic's span).
    Fuel { span: Span, out: Vec<u8> },
    /// A device-level failure surfaced as a REPL-layer diagnostic (P4): the
    /// peripheral reported an unbound aperture (E9152), carrying the aperture
    /// id when known. Distinct from a trap (trap numerics are frozen, I-3).
    /// Output already emitted stays emitted.
    Device {
        code: u32,
        span: Span,
        aperture: Option<u16>,
        out: Vec<u8>,
    },
    /// An engine-internal fault (E9158, or E9156's runtime backstop): a
    /// verified-IR-unreachable state. Always a bug — the page fails loudly
    /// (§6.5). Output already emitted stays emitted.
    Internal { fault: Fault, out: Vec<u8> },
}

/// An engine-internal fault (PLAN-WASM-1 §7.5, E9158): a state the *verified*
/// IR cannot reach (a missing block/target, a table mismatch between the
/// engine and the compiled artifact). Always an engine bug or an
/// engine/lowering contract break — surfaced as a REPL-layer diagnostic, so
/// the frozen trap numerics stay reserved for program-level traps (I-3).
/// `E9156` reuses this shape for its runtime backstop arm (tasks/interrupts
/// are rejected at compile; reaching the arm at all is recorded, not
/// mis-executed, FR-3).
#[derive(Clone, Debug, PartialEq)]
pub struct Fault {
    pub code: u32,
    pub span: Span,
    pub message: &'static [u8],
}

impl Fault {
    fn internal(span: Span, message: &'static [u8]) -> Fault {
        Fault {
            code: code::E9158_INTERNAL,
            span,
            message,
        }
    }

    fn tasks(span: Span) -> Fault {
        Fault {
            code: code::E9156_TASKS,
            span,
            message: b"task/interrupt op reached the engine (must be rejected at submit)",
        }
    }
}

// ---------------------------------------------------------------------------
// TrapDetail (PLAN-WASM-2 §Q6/DEC-22, §5.3): the runtime-diagnostic-protocol
// v1.1 additive optional `detail` field's payload. Closed `kind` set; every
// consumer MUST ignore an unknown kind (render "unknown detail", never guess).
// ---------------------------------------------------------------------------

/// The closed `kind` set of a [`TrapDetail`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailKind {
    /// A word's declared input contract vs the stack it actually observed —
    /// the L2a/L2c checked-pop/Call form, and the lowered `needs` guard.
    SigMismatch,
    /// A depth contract failed (reserved: call-depth/`StackBound` surfaces
    /// report frozen trap 10 today; the kind is part of the versioned set).
    Depth,
    /// A stack-bound contract failed (same reserved status as `Depth`).
    Bound,
}

impl DetailKind {
    /// The kind's wire name (protocol v1.1 §5.3).
    pub fn name(&self) -> &'static [u8] {
        match self {
            DetailKind::SigMismatch => b"sig-mismatch",
            DetailKind::Depth => b"depth",
            DetailKind::Bound => b"bound",
        }
    }
}

/// Which word declared what, and what the stack actually held at a contract
/// failure. `word`/`declared`/`actual` are present for `SigMismatch`; the
/// top-of-stack cells are capped (render-side caps are §Q9/DEC-25's job —
/// the engine emits at most `DETAIL_ACTUAL_CAP` cells).
#[derive(Clone, Debug, PartialEq)]
pub struct TrapDetail {
    pub kind: DetailKind,
    pub word: Vec<u8>,
    pub declared: Vec<u8>,
    pub actual: Vec<StackCell>,
}

/// The engine-side cap on a detail's `actual` cells (the stack *below* the
/// cap exists in ABI payloads — §Q9's data-side completeness rule).
pub const DETAIL_ACTUAL_CAP: usize = 8;

/// The execution state (§5).
#[derive(Clone, Debug)]
pub struct ExecState {
    /// The persistent data stack.
    pub stack: Vec<Slot>,
    /// Call frames.
    pub frames: Vec<Frame>,
    /// The region heap (§6.7).
    pub heap: RegionHeap,
    /// Diagnostic-channel bytes (the `testio.*` surface, §6.3).
    pub out: Vec<u8>,
    /// Op budget (DEC-10).
    pub fuel: Fuel,
    /// The undo journal / trace skeleton (§5, §7.3).
    pub journal: Journal,
    /// L1 guard elision (§Q4): when set, `TrapIfFalse` runs as a no-op (the
    /// emitted guard is skipped); L2/L3 are never affected.
    pub elide_guards: bool,
    /// The cursor of the next op to execute — the resume point (§Q6/§Q8:
    /// fuel exhaustion suspends here; `Engine::resume` continues from it).
    pub cursor: Option<Cursor>,
    /// Interned `ConstStr` addresses: `(span.start, addr)`.
    strings: Vec<(u32, u64)>,
    /// Resource-place storage: `(place bytes, addr)` — phrase-scoped (the
    /// rollback reclaims the interned reservations, so the cache must go).
    pub(crate) resources: Vec<(Vec<u8>, u64)>,
}

impl ExecState {
    /// A fresh state over a `Compiled` artifact: empty stack/frames, a fresh
    /// heap, the default fuel, and every `ConstStr` of the module interned
    /// (deterministic order → deterministic addresses, §Q6).
    pub fn new(compiled: &Compiled) -> ExecState {
        let mut heap = RegionHeap::new();
        let mut strings = Vec::new();
        for w in compiled.module.words.iter() {
            for b in w.blocks.iter() {
                for op in b.ops.iter() {
                    if let OpKind::ConstStr(span) = op.kind {
                        if strings.iter().any(|(s, _)| *s == span.start as u32) {
                            continue;
                        }
                        let bytes = compiled.src.get(span.start..span.end).unwrap_or(&[]);
                        if let Ok(addr) = heap.intern(bytes) {
                            strings.push((span.start as u32, addr));
                        }
                    }
                }
            }
        }
        ExecState {
            stack: Vec::new(),
            frames: Vec::new(),
            heap,
            out: Vec::new(),
            fuel: Fuel::new(crate::fuel::DEFAULT_FUEL),
            journal: Journal::new(),
            elide_guards: false,
            cursor: None,
            strings,
            resources: Vec::new(),
        }
    }

    /// A state seeded with the session's persistent stack + heap for one
    /// phrase run (the session holds the committed baseline; §7.3). `journal`
    /// and `out` come from the session's pool (D-W4): capacity persists
    /// across phrases; contents are cleared here.
    pub fn in_session(
        stack: Vec<Slot>,
        heap: RegionHeap,
        fuel: Fuel,
        elide_guards: bool,
        journal: Journal,
        out: Vec<u8>,
    ) -> ExecState {
        let mut journal = journal;
        journal.clear();
        let mut out = out;
        out.clear();
        ExecState {
            stack,
            frames: Vec::new(),
            heap,
            out,
            fuel,
            journal,
            elide_guards,
            cursor: None,
            strings: Vec::new(),
            resources: Vec::new(),
        }
    }

    /// Re-intern this compiled module's `ConstStr`s into the *current* heap
    /// (the session's persistent heap). Deterministic per `Compiled`.
    pub fn intern_compiled(&mut self, compiled: &Compiled) {
        for w in compiled.module.words.iter() {
            for b in w.blocks.iter() {
                for op in b.ops.iter() {
                    if let OpKind::ConstStr(span) = op.kind {
                        if self.strings.iter().any(|(s, _)| *s == span.start as u32) {
                            continue;
                        }
                        let bytes = compiled.src.get(span.start..span.end).unwrap_or(&[]);
                        if let Ok(addr) = self.heap.intern(bytes) {
                            self.strings.push((span.start as u32, addr));
                        }
                    }
                }
            }
        }
    }

    /// Record a diagnostic-channel write in the journal (trace-only).
    pub fn record_output(&mut self, len: u32) {
        if len == 0 {
            return;
        }
        let start: u32 = self
            .journal
            .iter()
            .filter_map(|e| match e {
                Effect::Output { len, .. } => Some(*len),
                _ => None,
            })
            .sum();
        self.journal.push(Effect::Output { start, len });
    }

    /// Address of an interned `ConstStr` (interned at construction).
    pub fn interned_string(&self, span: Span) -> Option<u64> {
        self.strings
            .iter()
            .find(|(s, _)| *s == span.start as u32)
            .map(|(_, a)| *a)
    }

    /// Address of a resource place: a heap-backed area allocated lazily (in
    /// first-access order — deterministic for a given run, §Q6). The
    /// reservation is journaled (DEC-5): a rollback reclaims it and drops
    /// the per-run cache (`resources` is phrase-scoped — see `rollback`).
    /// An arena-exhausted intern is a `RegionExhausted` trap — never a
    /// fabricated address (D-W5: the zero-default intern path is deleted).
    pub fn resource_addr(&mut self, place: &[u8]) -> Result<u64, TrapCode> {
        if let Some((_, a)) = self.resources.iter().find(|(p, _)| p == place) {
            return Ok(*a);
        }
        let old_cursor = self.heap.arena_cursor();
        match self.heap.intern(&[]) {
            Ok(addr) => {
                self.journal.push(Effect::Intern { addr, old_cursor });
                self.resources.push((place.to_vec(), addr));
                Ok(addr)
            }
            Err(_) => Err(TrapCode::RegionExhausted),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub word: usize,
    pub locals: Vec<Slot>,
    pub ret: Option<Cursor>,
}

/// The current (word, block, op) position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub word: usize,
    pub block: usize,
    pub op: usize,
}

/// The concrete run engine over a [`Compiled`] module.
pub struct Engine<'c> {
    pub compiled: &'c Compiled,
    pub spec: verifier::target::TargetSpec,
}

impl<'c> Engine<'c> {
    pub fn new(compiled: &'c Compiled) -> Engine<'c> {
        Engine {
            compiled,
            spec: compiled.spec,
        }
    }

    /// Run one word (by dictionary name) to completion. `periph` is the
    /// device boundary (I-1); the MMIO arms reach it when the session runs a
    /// module compiled with a platform descriptor (P4).
    pub fn run(&self, entry: &[u8], state: &mut ExecState, periph: &mut dyn Peripheral) -> Outcome {
        let Some(target) = self.compiled.dict_lookup(entry) else {
            return Outcome::Internal {
                fault: Fault::internal(Span::UNKNOWN, b"entry word missing from the dictionary"),
                out: core::mem::take(&mut state.out),
            };
        };
        let DictTarget::Word(word_idx) = target else {
            return Outcome::Internal {
                fault: Fault::internal(Span::UNKNOWN, b"entry name resolves to a builtin"),
                out: core::mem::take(&mut state.out),
            };
        };
        if !self.bound_ok(word_idx, state) {
            return Outcome::Trap {
                code: TrapCode::StackOverflow,
                span: Span::UNKNOWN,
                detail: None,
                out: core::mem::take(&mut state.out),
            };
        }
        state.frames.clear();
        state.frames.push(Frame {
            word: word_idx,
            locals: self.locals_for(word_idx),
            ret: None,
        });
        let cursor = Cursor {
            word: word_idx,
            block: self.entry_block(word_idx),
            op: 0,
        };
        self.drive(cursor, state, periph)
    }

    /// Resume a fuel-suspended run from `state.cursor` (§Q6/§Q8 — the
    /// stepping primitive). The state must be one that a previous
    /// `run`/`resume` left in `Outcome::Fuel`; the cursor is stored on the
    /// state at every step, so the engine carries no suspended-run registry.
    pub fn resume(&self, state: &mut ExecState, periph: &mut dyn Peripheral) -> Outcome {
        let Some(cursor) = state.cursor else {
            return Outcome::Internal {
                fault: Fault::internal(Span::UNKNOWN, b"resume without a stored cursor"),
                out: core::mem::take(&mut state.out),
            };
        };
        self.drive(cursor, state, periph)
    }

    /// The span of the op at `cur` (Span::UNKNOWN when the cursor itself is
    /// out of range — which `step` will fault on immediately after).
    fn op_span(&self, cur: Cursor) -> Span {
        self.compiled
            .module
            .words
            .get(cur.word)
            .and_then(|w| w.blocks.get(cur.block))
            .and_then(|b| b.ops.get(cur.op))
            .map(|op| op.span)
            .unwrap_or(Span::UNKNOWN)
    }

    fn drive(&self, cursor: Cursor, state: &mut ExecState, periph: &mut dyn Peripheral) -> Outcome {
        let mut cur = cursor;
        loop {
            // Record the resume point BEFORE consuming fuel: an exhaustion
            // return leaves `state.cursor` at the op that did not run (§Q8).
            state.cursor = Some(cur);
            if !state.fuel.take() {
                return Outcome::Fuel {
                    span: self.op_span(cur),
                    out: core::mem::take(&mut state.out),
                };
            }
            match self.step(cur, state, periph) {
                StepResult::Continue(next) => cur = next,
                StepResult::Done => {
                    return Outcome::Ok {
                        out: core::mem::take(&mut state.out),
                    };
                }
                StepResult::Trap(code, span) => {
                    return Outcome::Trap {
                        code,
                        span,
                        detail: None,
                        out: core::mem::take(&mut state.out),
                    };
                }
                StepResult::Contract(span, detail) => {
                    return Outcome::Trap {
                        code: TrapCode::ContractFail,
                        span,
                        detail: Some(detail),
                        out: core::mem::take(&mut state.out),
                    };
                }
                StepResult::Exit(code) => {
                    return Outcome::Exited {
                        code,
                        out: core::mem::take(&mut state.out),
                    };
                }
                StepResult::Fault(fault) => {
                    return Outcome::Internal {
                        fault,
                        out: core::mem::take(&mut state.out),
                    };
                }
                StepResult::Device(code, span, aperture) => {
                    return Outcome::Device {
                        code,
                        span,
                        aperture,
                        out: core::mem::take(&mut state.out),
                    };
                }
            }
        }
    }

    fn step(&self, cur: Cursor, state: &mut ExecState, periph: &mut dyn Peripheral) -> StepResult {
        let word = &self.compiled.module.words[cur.word];
        let Some(block) = word.blocks.get(cur.block) else {
            return StepResult::Fault(Fault::internal(
                Span::UNKNOWN,
                b"cursor block out of range (verified IR cannot reach this)",
            ));
        };
        let Some(op) = block.ops.get(cur.op) else {
            return StepResult::Fault(Fault::internal(
                Span::UNKNOWN,
                b"cursor op out of range (verified IR cannot reach this)",
            ));
        };
        let next_op = Cursor {
            word: cur.word,
            block: cur.block,
            op: cur.op + 1,
        };
        match op.kind {
            OpKind::ConstI64(v) => {
                state.stack.push(Slot::int(ir::TY_I64, v));
                StepResult::Continue(next_op)
            }
            OpKind::ConstBool(b) => {
                state.stack.push(Slot::bool(b));
                StepResult::Continue(next_op)
            }
            OpKind::ConstStr(span) => match state.interned_string(span) {
                Some(addr) => {
                    state.stack.push(Slot::ptr(ir::TY_STR, addr));
                    StepResult::Continue(next_op)
                }
                None => StepResult::Trap(TrapCode::RegionExhausted, op.span),
            },
            OpKind::AddrOf {
                place,
                mutable: _,
                base,
            } => match base {
                ir::AddrOfBase::Runtime => match state.resource_addr(place.as_bytes()) {
                    Ok(addr) => {
                        state.stack.push(Slot::ptr(ir::TY_PTR_MUT, addr));
                        StepResult::Continue(next_op)
                    }
                    // Arena exhausted: a real trap, never a fabricated address
                    // (D-W5 — the zero-default intern path is deleted).
                    Err(t) => StepResult::Trap(t, op.span),
                },
                ir::AddrOfBase::Mmio { aperture, offset } => {
                    // A register address: the aperture's bound base plus the
                    // register offset (P4). Resolved through the compiled
                    // aperture table (from the platform descriptor).
                    let addr = self.mmio_base(u64::from(aperture), u64::from(offset));
                    state.stack.push(Slot::ptr(ir::TY_PTR_MUT, addr));
                    StepResult::Continue(next_op)
                }
            },
            OpKind::MmioPlace {
                place: _,
                aperture,
                offset,
            } => {
                // A register-map instance place (a *field* of a register-map
                // const) addresses the aperture directly — no `&` needed. The
                // pushed address is exactly the AddrOf{Mmio} address above.
                let addr = self.mmio_base(u64::from(aperture), u64::from(offset));
                state.stack.push(Slot::ptr(ir::TY_PTR_MUT, addr));
                StepResult::Continue(next_op)
            }
            OpKind::ScopedEnter { ty, len } => match self.step_scoped_enter(word, ty, len, state) {
                Ok(()) => StepResult::Continue(next_op),
                Err(fault) => StepResult::Trap(fault, op.span),
            },
            OpKind::TaskSpawn { .. } => StepResult::Fault(Fault::tasks(op.span)),
            OpKind::PtrAddConst { offset, .. } => match pop_checked(state, ValKind::Ptr) {
                Ok(p) => {
                    let Value::Ptr(base) = p.val else {
                        unreachable!("pop_checked(Ptr) returned a non-pointer")
                    };
                    state.stack.push(Slot::ptr(
                        ir::TY_PTR_MUT,
                        base.wrapping_add(u64::from(offset)),
                    ));
                    StepResult::Continue(next_op)
                }
                Err(f) => self.pop_fault(f, op.span, word, ValKind::Ptr),
            },
            OpKind::PtrAddIndex { scale, .. } => {
                match (
                    pop_checked(state, ValKind::Int),
                    pop_checked(state, ValKind::Ptr),
                ) {
                    (Ok(idx), Ok(base)) => {
                        let Value::Int(i) = idx.val else {
                            unreachable!("pop_checked(Int) returned a non-int")
                        };
                        let Value::Ptr(bp) = base.val else {
                            unreachable!("pop_checked(Ptr) returned a non-pointer")
                        };
                        let addr = bp.wrapping_add((i as u64).wrapping_mul(u64::from(scale)));
                        state.stack.push(Slot::ptr(ir::TY_PTR_MUT, addr));
                        StepResult::Continue(next_op)
                    }
                    (Err(f), _) => self.pop_fault(f, op.span, word, ValKind::Int),
                    (_, Err(f)) => self.pop_fault(f, op.span, word, ValKind::Ptr),
                }
            }
            OpKind::Dup { .. } => {
                // Single-slot reference/value copy (native `emit_dup`). Empty
                // stack is a real underflow (trap 10) — never a fabricated
                // zero (D-W5).
                match state.stack.last() {
                    Some(&top) => {
                        state.stack.push(top);
                        StepResult::Continue(next_op)
                    }
                    None => StepResult::Trap(TrapCode::StackOverflow, op.span),
                }
            }
            OpKind::Drop { .. } => match pop_any(state) {
                Ok(_) => StepResult::Continue(next_op),
                Err(f) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::Swap { .. } => {
                let n = state.stack.len();
                if n >= 2 {
                    state.stack.swap(n - 1, n - 2);
                    StepResult::Continue(next_op)
                } else {
                    // A silent no-op on underflow is deleted (D-W5).
                    StepResult::Trap(TrapCode::StackOverflow, op.span)
                }
            }
            OpKind::AddI64 => match (
                pop_checked(state, ValKind::Int),
                pop_checked(state, ValKind::Int),
            ) {
                (Ok(b), Ok(a)) => {
                    let Value::Int(bv) = b.val else {
                        unreachable!("pop_checked(Int) returned a non-int")
                    };
                    let Value::Int(av) = a.val else {
                        unreachable!("pop_checked(Int) returned a non-int")
                    };
                    // G53 red-path instrument: under `wasm-selftest` this arm
                    // subtracts so the execution differential MUST fail.
                    let r = if selftest_corrupt_add() {
                        av.wrapping_sub(bv)
                    } else {
                        av.wrapping_add(bv)
                    };
                    state.stack.push(Slot::int(ir::TY_I64, r));
                    StepResult::Continue(next_op)
                }
                (Err(f), _) | (_, Err(f)) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::SubI64 => match (
                pop_checked(state, ValKind::Int),
                pop_checked(state, ValKind::Int),
            ) {
                (Ok(b), Ok(a)) => {
                    let (Value::Int(bv), Value::Int(av)) = (b.val, a.val) else {
                        unreachable!("pop_checked(Int) returned a non-int")
                    };
                    state.stack.push(Slot::int(ir::TY_I64, av.wrapping_sub(bv)));
                    StepResult::Continue(next_op)
                }
                (Err(f), _) | (_, Err(f)) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::MulI64 => match (
                pop_checked(state, ValKind::Int),
                pop_checked(state, ValKind::Int),
            ) {
                (Ok(b), Ok(a)) => {
                    let (Value::Int(bv), Value::Int(av)) = (b.val, a.val) else {
                        unreachable!("pop_checked(Int) returned a non-int")
                    };
                    state.stack.push(Slot::int(ir::TY_I64, av.wrapping_mul(bv)));
                    StepResult::Continue(next_op)
                }
                (Err(f), _) | (_, Err(f)) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::Cmp { out, kind } => match (
                pop_checked(state, ValKind::Int),
                pop_checked(state, ValKind::Int),
            ) {
                (Ok(b), Ok(a)) => {
                    let (Value::Int(bv), Value::Int(av)) = (b.val, a.val) else {
                        unreachable!("pop_checked(Int) returned a non-int")
                    };
                    let r = match kind {
                        ir::CmpKind::Lt => av < bv,
                        ir::CmpKind::Le => av <= bv,
                        ir::CmpKind::Gt => av > bv,
                        ir::CmpKind::Ge => av >= bv,
                        ir::CmpKind::Eq => av == bv,
                        ir::CmpKind::Ne => av != bv,
                    };
                    state.stack.push(Slot {
                        ty: out,
                        val: Value::Bool(r),
                    });
                    StepResult::Continue(next_op)
                }
                (Err(f), _) | (_, Err(f)) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::AndBool => {
                match (
                    pop_checked(state, ValKind::Bool),
                    pop_checked(state, ValKind::Bool),
                ) {
                    (Ok(b), Ok(a)) => {
                        let (Value::Bool(bv), Value::Bool(av)) = (b.val, a.val) else {
                            unreachable!("pop_checked(Bool) returned a non-bool")
                        };
                        state.stack.push(Slot::bool(av && bv));
                        StepResult::Continue(next_op)
                    }
                    (Err(f), _) | (_, Err(f)) => self.pop_fault(f, op.span, word, ValKind::Bool),
                }
            }
            OpKind::OrBool => {
                match (
                    pop_checked(state, ValKind::Bool),
                    pop_checked(state, ValKind::Bool),
                ) {
                    (Ok(b), Ok(a)) => {
                        let (Value::Bool(bv), Value::Bool(av)) = (b.val, a.val) else {
                            unreachable!("pop_checked(Bool) returned a non-bool")
                        };
                        state.stack.push(Slot::bool(av || bv));
                        StepResult::Continue(next_op)
                    }
                    (Err(f), _) | (_, Err(f)) => self.pop_fault(f, op.span, word, ValKind::Bool),
                }
            }
            OpKind::NotBool => match pop_checked(state, ValKind::Bool) {
                Ok(a) => {
                    let Value::Bool(av) = a.val else {
                        unreachable!("pop_checked(Bool) returned a non-bool")
                    };
                    state.stack.push(Slot::bool(!av));
                    StepResult::Continue(next_op)
                }
                Err(f) => self.pop_fault(f, op.span, word, ValKind::Bool),
            },
            OpKind::InterruptDisable | OpKind::InterruptEnable => {
                StepResult::Fault(Fault::tasks(op.span))
            }
            OpKind::LocalSet { slot, ty: _ } => match pop_any(state) {
                Ok(v) => {
                    match state.frames.last_mut() {
                        Some(f) => {
                            let idx = slot as usize;
                            if f.locals.len() <= idx {
                                f.locals.resize(idx + 1, Slot::int(ir::TY_I64, 0));
                            }
                            f.locals[idx] = v;
                        }
                        // A store with no frame is a real underflow (trap 10),
                        // never a silent drop (D-W5).
                        None => return StepResult::Trap(TrapCode::StackOverflow, op.span),
                    }
                    StepResult::Continue(next_op)
                }
                Err(f) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::LocalGet { slot, ty: _ } => {
                // A missing frame/local is a real underflow (trap 10) — the
                // fabricated-zero path is deleted (D-W5). The local's own type
                // travels with it (set by the LocalSet that wrote it).
                match state
                    .frames
                    .last()
                    .and_then(|f| f.locals.get(slot as usize))
                    .copied()
                {
                    Some(v) => {
                        state.stack.push(v);
                        StepResult::Continue(next_op)
                    }
                    None => StepResult::Trap(TrapCode::StackOverflow, op.span),
                }
            }
            OpKind::Cast { from, to } => match pop_any(state) {
                Ok(v) => match self.step_cast(word, from, to, v) {
                    Ok(out_slot) => {
                        state.stack.push(out_slot);
                        StepResult::Continue(next_op)
                    }
                    Err(()) => StepResult::Trap(TrapCode::SubtypeFail, op.span),
                },
                Err(f) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::Bitcast { to, .. } => match pop_any(state) {
                Ok(v) => {
                    // No range check (§Q7's resolved reading: the checker
                    // admits a bitcast exactly where no runtime check is
                    // needed); the value retags and L2 tracking follows it.
                    state.stack.push(Slot { ty: to, val: v.val });
                    StepResult::Continue(next_op)
                }
                Err(f) => self.pop_fault(f, op.span, word, ValKind::Int),
            },
            OpKind::Call { name, sig, .. } => match self.compiled.dict_lookup(name.as_bytes()) {
                Some(DictTarget::Builtin(b)) => {
                    // L2c, builtin form: the interpreter builtins declare their
                    // argument kinds; a mismatch is trap 20 with detail — the
                    // same defense-in-depth as word calls (never a coercion).
                    if let Err(fault) = b.check_args(state) {
                        return match fault {
                            None => StepResult::Trap(TrapCode::StackOverflow, op.span),
                            Some(want) => StepResult::Contract(
                                op.span,
                                TrapDetail {
                                    kind: DetailKind::SigMismatch,
                                    word: name.as_bytes().to_vec(),
                                    declared: want.name().to_vec(),
                                    actual: top_cells(state, word, DETAIL_ACTUAL_CAP),
                                },
                            ),
                        };
                    }
                    match self.call_builtin(b, state) {
                        Ok(()) => {
                            // Retag the pushed outputs to the declared
                            // signature: builtins push raw prim-tagged slots
                            // (e.g. a region handle as `i64`), but the
                            // checker verified this call site against the
                            // import table's declared outputs — those are
                            // exact, and the L2 block-entry checks downstream
                            // compare against them.
                            let out_len = sig.out_len as usize;
                            if out_len > 0 {
                                let base = state.stack.len() - out_len;
                                for i in 0..out_len {
                                    state.stack[base + i].ty = sig.outputs[i];
                                }
                            }
                            StepResult::Continue(next_op)
                        }
                        Err(BuiltinSignal::Exit(code)) => StepResult::Exit(code),
                        Err(BuiltinSignal::Trap(code)) => StepResult::Trap(code, op.span),
                    }
                }
                Some(DictTarget::Word(callee)) => {
                    // L2: stack depth ≥ sig.in_len (never elidable). Depth
                    // shortfall remains trap 10 — underflow is the stack
                    // error (PLAN-WASM-1 §7.8).
                    if state.stack.len() < sig.in_len as usize {
                        return StepResult::Trap(TrapCode::StackOverflow, op.span);
                    }
                    if state.frames.len() + 1 >= CALL_DEPTH_CAP {
                        return StepResult::Trap(TrapCode::StackOverflow, op.span);
                    }
                    if !self.bound_ok(callee, state) {
                        return StepResult::Trap(TrapCode::StackOverflow, op.span);
                    }
                    // L2c (DEC-23): per-slot input contract. Type ids are
                    // word-local (each `ir::Word` interns its own table; the
                    // prim slots 0..6 are the only globally fixed ids), so the
                    // check compares *atoms* through each word's table — and
                    // on success retags each incoming slot into the callee's
                    // table (ids are interpreted per current word; the retag
                    // is what keeps L2 tracking consistent across the call).
                    let callee_word = &self.compiled.module.words[callee];
                    let in_len = sig.in_len as usize;
                    let base = state.stack.len() - in_len;
                    for i in 0..in_len {
                        let want = sig.inputs[i];
                        let slot = state.stack[base + i];
                        let Some(caller_atom) = word.types.get(slot.ty.0 as usize) else {
                            return StepResult::Fault(Fault::internal(
                                op.span,
                                b"argument type id out of the caller's type table (engine bug)",
                            ));
                        };
                        let Some(callee_id) = callee_word
                            .types
                            .iter()
                            .position(|a| a.as_bytes() == caller_atom.as_bytes())
                            .map(|p| ir::TypeId(p as u8))
                        else {
                            return StepResult::Fault(Fault::internal(
                                op.span,
                                b"argument type missing from the callee's type table (engine bug)",
                            ));
                        };
                        if callee_id != want {
                            let actual = state.stack[base..]
                                .iter()
                                .take(DETAIL_ACTUAL_CAP)
                                .map(|&s| cell_of(word, s))
                                .collect();
                            return StepResult::Contract(
                                op.span,
                                TrapDetail {
                                    kind: DetailKind::SigMismatch,
                                    word: callee_word.name.as_bytes().to_vec(),
                                    declared: sig_text_of(callee_word),
                                    actual,
                                },
                            );
                        }
                        state.stack[base + i].ty = callee_id;
                    }
                    let caller = Cursor {
                        word: cur.word,
                        block: cur.block,
                        op: cur.op + 1,
                    };
                    let depth_before = state.frames.len() as u16;
                    state.frames.push(Frame {
                        word: callee,
                        locals: self.locals_for(callee),
                        // A frame's `ret` is the address to resume after THIS
                        // frame returns; the entry frame's is `None` (its
                        // return completes the run).
                        ret: Some(caller),
                    });
                    // Journal the call (trace-only; the depth-bounded render
                    // is the §7.4 normalization's job).
                    let name = self.compiled.module.words[callee].name.as_bytes().to_vec();
                    state.journal.push(Effect::Call {
                        word: name,
                        depth_before,
                    });
                    let callee_entry = self.entry_block(callee);
                    // L2b (DEC-23) at the callee's entry block: the runtime
                    // top-of-stack types must equal the block's checked
                    // `entry_stack`. Ids are consistent here — the inputs were
                    // just retagged into the callee's table. A mismatch is
                    // *definitionally* an engine bug (§Q7's reachability
                    // argument): post-typecheck IR cannot express it, so it
                    // pages fails as E9158, never as a student-facing trap.
                    if let Err(e) = self.check_block_entry(callee, callee_entry, state, op.span) {
                        return e;
                    }
                    StepResult::Continue(Cursor {
                        word: callee,
                        block: callee_entry,
                        op: 0,
                    })
                }
                None => StepResult::Fault(Fault::internal(
                    op.span,
                    b"call target out of range (dictionary resolution guarantees a word)",
                )),
            },
            OpKind::Load { ty } => match self.step_load(word, ty, op.span, state) {
                Ok(()) => StepResult::Continue(next_op),
                Err(e) => e,
            },
            OpKind::Store { ty } => match self.step_store(word, ty, op.span, state) {
                Ok(()) => StepResult::Continue(next_op),
                Err(e) => e,
            },
            OpKind::MmioVolLoad {
                place: _,
                ty,
                read_kind,
                atomic_max: _,
                barrier,
            } => {
                let _ = (read_kind, barrier); // single-threaded: Effectful/Barrier
                match self.step_mmio_load(word, ty, op.span, state, periph) {
                    Ok(()) => StepResult::Continue(next_op),
                    Err(e) => e,
                }
            }
            OpKind::MmioVolStore {
                place: _,
                ty,
                write_kind,
                read_kind,
                atomic_max: _,
                barrier,
            } => {
                let _ = (read_kind, barrier); // single-threaded: Effectful/Barrier
                match self.step_mmio_store(word, ty, write_kind, op.span, state, periph) {
                    Ok(()) => StepResult::Continue(next_op),
                    Err(e) => e,
                }
            }
            OpKind::MmioVolLoadField {
                place: _,
                read_kind,
                barrier,
                ..
            } => {
                let _ = (read_kind, barrier);
                match self.step_mmio_load_field(word, op, state, periph) {
                    Ok(()) => StepResult::Continue(next_op),
                    Err(e) => e,
                }
            }
            OpKind::MmioVolStoreField {
                place: _,
                read_kind,
                barrier,
                ..
            } => {
                let _ = (read_kind, barrier);
                match self.step_mmio_store_field(word, op, state, periph) {
                    Ok(()) => StepResult::Continue(next_op),
                    Err(e) => e,
                }
            }
            OpKind::TrapIfFalse { code } => match pop_checked(state, ValKind::Bool) {
                Err(f) => self.pop_fault(f, op.span, word, ValKind::Bool),
                Ok(slot) => {
                    let Value::Bool(b) = slot.val else {
                        unreachable!("pop_checked(Bool) returned a non-bool")
                    };
                    if state.elide_guards {
                        // L1 elision (§Q4): the emitted guard is skipped — the
                        // bool is consumed, control falls through. L2/L3
                        // interpreter invariants are unaffected.
                        StepResult::Continue(next_op)
                    } else if b {
                        StepResult::Continue(next_op)
                    } else if code == TrapCode::ContractFail {
                        // A lowered `needs` guard failed (L1, trap 20): the
                        // §Q6 detail names the word whose contract failed, its
                        // declared signature, and the stack it observed.
                        StepResult::Contract(
                            op.span,
                            TrapDetail {
                                kind: DetailKind::SigMismatch,
                                word: word.name.as_bytes().to_vec(),
                                declared: sig_text_of(word),
                                actual: top_cells(state, word, DETAIL_ACTUAL_CAP),
                            },
                        )
                    } else {
                        StepResult::Trap(code, op.span)
                    }
                }
            },
            OpKind::Br { target } => {
                let b = match self.block_index(word, target) {
                    Some(b) => b,
                    None => {
                        return StepResult::Fault(Fault::internal(
                            op.span,
                            b"branch target out of range (verified IR cannot reach this)",
                        ))
                    }
                };
                // L2b (DEC-23) at the branch target — same contract as the
                // callee entry above (mismatch = engine bug = E9158).
                if let Err(e) = self.check_block_entry(cur.word, b, state, op.span) {
                    return e;
                }
                StepResult::Continue(Cursor {
                    word: cur.word,
                    block: b,
                    op: 0,
                })
            }
            OpKind::BrIf { then_tgt, else_tgt } => {
                let target = match pop_checked(state, ValKind::Bool) {
                    Ok(slot) => {
                        let Value::Bool(b) = slot.val else {
                            unreachable!("pop_checked(Bool) returned a non-bool")
                        };
                        if b {
                            then_tgt
                        } else {
                            else_tgt
                        }
                    }
                    // An empty stack at a conditional branch is a real
                    // underflow (trap 10) — the silent-false path is deleted
                    // (D-W5); a kind mismatch is the L2a contract trap.
                    Err(PopFault::Underflow) => {
                        return StepResult::Trap(TrapCode::StackOverflow, op.span)
                    }
                    Err(f) => return self.pop_fault(f, op.span, word, ValKind::Bool),
                };
                let b = match self.block_index(word, target) {
                    Some(b) => b,
                    None => {
                        return StepResult::Fault(Fault::internal(
                            op.span,
                            b"branch target out of range (verified IR cannot reach this)",
                        ))
                    }
                };
                if let Err(e) = self.check_block_entry(cur.word, b, state, op.span) {
                    return e;
                }
                StepResult::Continue(Cursor {
                    word: cur.word,
                    block: b,
                    op: 0,
                })
            }
            OpKind::Ret => {
                let Some(frame) = state.frames.pop() else {
                    return StepResult::Done;
                };
                // Retag the frame's outputs into the CALLER's type table (ids
                // are word-local; this is the return-side half of the Call
                // entry retag). The entry frame's pop ends the run (Done).
                if frame.ret.is_some() {
                    let callee = &self.compiled.module.words[cur.word];
                    let out_len = callee.sig.out_len as usize;
                    if out_len > 0 {
                        let Some(caller_idx) = state.frames.last().map(|f| f.word) else {
                            return StepResult::Fault(Fault::internal(
                                op.span,
                                b"return without a caller frame (engine bug)",
                            ));
                        };
                        let caller = &self.compiled.module.words[caller_idx];
                        let base = state.stack.len().saturating_sub(out_len);
                        for slot in state.stack[base..].iter_mut() {
                            let Some(atom) = callee.types.get(slot.ty.0 as usize) else {
                                return StepResult::Fault(Fault::internal(
                                    op.span,
                                    b"return type id out of the callee's type table (engine bug)",
                                ));
                            };
                            let bytes = atom.as_bytes();
                            let Some(pos) = caller.types.iter().position(|a| a.as_bytes() == bytes)
                            else {
                                return StepResult::Fault(Fault::internal(
                                    op.span,
                                    b"return type missing from the caller's type table (engine bug)",
                                ));
                            };
                            slot.ty = ir::TypeId(pos as u8);
                        }
                    }
                }
                match frame.ret {
                    Some(ret) => StepResult::Continue(ret),
                    None => StepResult::Done,
                }
            }
        }
    }

    fn step_scoped_enter(
        &self,
        word: &ir::Word,
        ty: ir::TypeId,
        len: u32,
        state: &mut ExecState,
    ) -> Result<(), TrapCode> {
        match word.type_classes.get(ty.0 as usize).copied() {
            // Slice: a `{ptr,len}` descriptor whose address is pushed (native
            // stores it in the frame's scoped area; the heap is the concrete
            // arena here, §6.7).
            Some(ir::TypeClass::Slice) => {
                // The descriptor's base is the pointer on top of the stack —
                // a checked peek (L2a): underflow is trap 10, a non-pointer
                // top is the contract trap. No fabricated zero (D-W5).
                let top = match state.stack.last() {
                    Some(&s) => s,
                    None => return Err(TrapCode::StackOverflow),
                };
                let Value::Ptr(base) = top.val else {
                    // The caller needs the StepResult shape, so the contract
                    // fault is reported through the scoped-enter error path's
                    // trap channel with the op's span.
                    return Err(TrapCode::ContractFail);
                };
                let (addr, old_cursor) = state
                    .heap
                    .scoped_enter(base, u64::from(len))
                    .map_err(|f| f.trap())?;
                state.stack.push(Slot::ptr(ty, addr));
                // Journal the descriptor allocation so a rollback drops its
                // live range and reclaims the span (§7.3).
                state.journal.push(Effect::ScopedEnter { addr, old_cursor });
                Ok(())
            }
            Some(ir::TypeClass::RegionRef | ir::TypeClass::RegionRefMut) => {
                match state.stack.last() {
                    Some(&top) => {
                        state.stack.push(top);
                        Ok(())
                    }
                    None => Err(TrapCode::StackOverflow),
                }
            }
            _ => Err(TrapCode::Unreachable),
        }
    }

    /// L3: `Cast` re-tags the value; for an integer subtype with a known
    /// range, out-of-range is trap 21 (`SubtypeFail`) — never elidable (§Q4).
    fn step_cast(
        &self,
        word: &ir::Word,
        _from: ir::TypeId,
        to: ir::TypeId,
        v: Slot,
    ) -> Result<Slot, ()> {
        let base = word
            .subtype_bases
            .get(to.0 as usize)
            .copied()
            .unwrap_or(ir::TY_EMPTY);
        if base != ir::TY_EMPTY {
            if let Some(name) = word.types.get(to.0 as usize) {
                if let Some(range) = self.compiled.subtype_range(name.as_bytes()) {
                    if let Value::Int(val) = v.val {
                        if val < range.min || val > range.max {
                            return Err(());
                        }
                    }
                }
            }
        }
        Ok(Slot { ty: to, val: v.val })
    }

    fn step_load(
        &self,
        word: &ir::Word,
        ty: ir::TypeId,
        span: Span,
        state: &mut ExecState,
    ) -> Result<(), StepResult> {
        let (bits, signed) = ir::storage_bits_signed(word, ty, self.spec.word_bits as u16)
            .ok_or_else(|| {
                StepResult::Fault(Fault::internal(span, b"load type has no storage encoding"))
            })?;
        let width = bits_to_width(bits);
        let addr = self.pop_ptr_or(state, span, word)?;
        let raw = state
            .heap
            .load(addr, width)
            .map_err(|f| StepResult::Trap(f.trap(), span))?;
        let v = match (width, signed) {
            (1, true) => (raw as u8 as i8) as i64,
            (1, false) => (raw as u8) as i64,
            (2, true) => (raw as u16 as i16) as i64,
            (2, false) => (raw as u16) as i64,
            (4, true) => (raw as u32 as i32) as i64,
            (4, false) => raw as u32 as i64,
            (8, _) => raw as i64,
            _ => {
                return Err(StepResult::Fault(Fault::internal(
                    span,
                    b"unreachable storage width",
                )))
            }
        };
        state.stack.push(Slot::int(ty, v));
        Ok(())
    }

    fn step_store(
        &self,
        word: &ir::Word,
        ty: ir::TypeId,
        span: Span,
        state: &mut ExecState,
    ) -> Result<(), StepResult> {
        let (bits, _) =
            ir::storage_bits_signed(word, ty, self.spec.word_bits as u16).ok_or_else(|| {
                StepResult::Fault(Fault::internal(span, b"store type has no storage encoding"))
            })?;
        let width = bits_to_width(bits);
        let v = self.pop_int_or(state, span, word)?;
        let addr = self.pop_ptr_or(state, span, word)?;
        let old = state
            .heap
            .store(addr, width, v as u64)
            .map_err(|f| StepResult::Trap(f.trap(), span))?;
        // Journal the store's old bytes — the transaction's heap undo (§7.3).
        state.journal.push(Effect::Store { addr, width, old });
        Ok(())
    }

    // -- P4 MMIO -------------------------------------------------------------

    /// The address an `AddrOf{Mmio}`/`MmioPlace` pushes: the aperture's bound
    /// base plus the register offset (through the compiled aperture table).
    fn mmio_base(&self, aperture: u64, offset: u64) -> u64 {
        self.compiled
            .apertures
            .iter()
            .find(|a| u64::from(a.id) == aperture)
            .map(|a| a.base.wrapping_add(offset))
            .unwrap_or(offset)
    }

    /// Resolve a pushed MMIO address to `(aperture, register_offset)` via the
    /// aperture table. `None` = the address is not inside any bound aperture
    /// (E9152 — no device/aperture for it).
    fn mmio_target(&self, addr: u64) -> Option<(u16, u32)> {
        self.compiled
            .apertures
            .iter()
            .find(|a| {
                let end = a.base.wrapping_add(u64::from(a.size));
                addr >= a.base && addr < end
            })
            .map(|a| (a.id, (addr - a.base) as u32))
    }

    /// The register-cell width of a typed value (the integer storage width).
    fn mmio_width(&self, word: &ir::Word, ty: ir::TypeId) -> Option<u8> {
        let (bits, _) = ir::storage_bits_signed(word, ty, self.spec.word_bits as u16)?;
        Some(bits_to_width(bits))
    }

    /// E9152 for an unbound aperture / outside-aperture address (P4), carrying
    /// the aperture id when known.
    fn device_err(span: Span, aperture: Option<u16>) -> StepResult {
        StepResult::Device(code::E9152_PERIPH_UNBOUND, span, aperture)
    }

    /// `MmioVolLoad`: pop the register address, read the device, push the
    /// value (width-masked by the device).
    fn step_mmio_load(
        &self,
        word: &ir::Word,
        ty: ir::TypeId,
        span: Span,
        state: &mut ExecState,
        periph: &mut dyn Peripheral,
    ) -> Result<(), StepResult> {
        let width = self.mmio_width(word, ty).ok_or_else(|| {
            StepResult::Fault(Fault::internal(
                span,
                b"mmio register width unresolved (descriptor/module mismatch)",
            ))
        })?;
        let addr = self.pop_ptr_or(state, span, word)?;
        let (ap, off) = self
            .mmio_target(addr)
            .ok_or_else(|| Self::device_err(span, None))?;
        let v = periph
            .read(ap, off, width)
            .map_err(|_| Self::device_err(span, Some(ap)))?;
        state.stack.push(Slot::int(ty, v as i64));
        state.journal.push(Effect::Periph {
            aperture: ap,
            off,
            width,
            val: v,
            is_write: false,
        });
        Ok(())
    }

    /// `MmioVolStore`: `WriteKind` semantics — a `Plain` store writes the
    /// width-masked value; `W1c`/`W1s`/`Xor` read the register (RMW), apply
    /// the bit math, and write the result.
    fn step_mmio_store(
        &self,
        word: &ir::Word,
        ty: ir::TypeId,
        write_kind: ir::WriteKind,
        span: Span,
        state: &mut ExecState,
        periph: &mut dyn Peripheral,
    ) -> Result<(), StepResult> {
        let width = self.mmio_width(word, ty).ok_or_else(|| {
            StepResult::Fault(Fault::internal(
                span,
                b"mmio register width unresolved (descriptor/module mismatch)",
            ))
        })?;
        let v = self.pop_int_or(state, span, word)?;
        let addr = self.pop_ptr_or(state, span, word)?;
        let (ap, off) = self
            .mmio_target(addr)
            .ok_or_else(|| Self::device_err(span, None))?;
        let out = if write_kind == ir::WriteKind::Plain {
            v as u64 & width_mask(width)
        } else {
            let cur = periph
                .read(ap, off, width)
                .map_err(|_| Self::device_err(span, Some(ap)))?;
            apply_write_kind(cur, v as u64, width, write_kind)
        };
        periph
            .write(ap, off, width, out)
            .map_err(|_| Self::device_err(span, Some(ap)))?;
        state.journal.push(Effect::Periph {
            aperture: ap,
            off,
            width,
            val: out,
            is_write: true,
        });
        Ok(())
    }

    /// `MmioVolLoadField`: read the register, mask/shift out the field.
    fn step_mmio_load_field(
        &self,
        word: &ir::Word,
        op: &ir::Op,
        state: &mut ExecState,
        periph: &mut dyn Peripheral,
    ) -> Result<(), StepResult> {
        let OpKind::MmioVolLoadField {
            reg_ty,
            field_ty,
            mask,
            shift,
            ..
        } = op.kind
        else {
            return Err(StepResult::Fault(Fault::internal(
                op.span,
                b"op kind mismatch on the mmio field path",
            )));
        };
        let span = op.span;
        let width = self.mmio_width(word, reg_ty).ok_or_else(|| {
            StepResult::Fault(Fault::internal(
                span,
                b"mmio register width unresolved (descriptor/module mismatch)",
            ))
        })?;
        let addr = self.pop_ptr_or(state, span, word)?;
        let (ap, off) = self
            .mmio_target(addr)
            .ok_or_else(|| Self::device_err(span, None))?;
        let reg = periph
            .read(ap, off, width)
            .map_err(|_| Self::device_err(span, Some(ap)))?;
        let field = extract_field(reg, mask, shift);
        state.stack.push(Slot::int(field_ty, field as i64));
        state.journal.push(Effect::Periph {
            aperture: ap,
            off,
            width,
            val: reg,
            is_write: false,
        });
        Ok(())
    }

    /// `MmioVolStoreField`: RMW the field into the register, then apply the
    /// `WriteKind` to the register write (Plain = the field RMW result).
    fn step_mmio_store_field(
        &self,
        word: &ir::Word,
        op: &ir::Op,
        state: &mut ExecState,
        periph: &mut dyn Peripheral,
    ) -> Result<(), StepResult> {
        let OpKind::MmioVolStoreField {
            reg_ty,
            mask,
            shift,
            write_kind,
            ..
        } = op.kind
        else {
            return Err(StepResult::Fault(Fault::internal(
                op.span,
                b"op kind mismatch on the mmio field path",
            )));
        };
        let span = op.span;
        let width = self.mmio_width(word, reg_ty).ok_or_else(|| {
            StepResult::Fault(Fault::internal(
                span,
                b"mmio register width unresolved (descriptor/module mismatch)",
            ))
        })?;
        let field = self.pop_int_or(state, span, word)?;
        let addr = self.pop_ptr_or(state, span, word)?;
        let (ap, off) = self
            .mmio_target(addr)
            .ok_or_else(|| Self::device_err(span, None))?;
        let cur = periph
            .read(ap, off, width)
            .map_err(|_| Self::device_err(span, Some(ap)))?;
        let reg = insert_field(cur, mask, shift, field as u64);
        let out = if write_kind == ir::WriteKind::Plain {
            reg
        } else {
            apply_write_kind(cur, reg, width, write_kind)
        };
        periph
            .write(ap, off, width, out)
            .map_err(|_| Self::device_err(span, Some(ap)))?;
        state.journal.push(Effect::Periph {
            aperture: ap,
            off,
            width,
            val: out,
            is_write: true,
        });
        Ok(())
    }

    fn call_builtin(&self, b: Builtin, state: &mut ExecState) -> Result<(), BuiltinSignal> {
        b.execute(state)
    }

    /// L2 `StackBound` depth check (never elidable): the word's declared peak
    /// (`high`) must fit within the stack cap given the current depth.
    fn bound_ok(&self, word_idx: usize, state: &ExecState) -> bool {
        let word = &self.compiled.module.words[word_idx];
        match word.bound.high {
            ir::High::Top => true,
            ir::High::Slots(extra) => state.stack.len().saturating_add(extra as usize) <= STACK_CAP,
        }
    }

    fn locals_for(&self, word_idx: usize) -> Vec<Slot> {
        let word = &self.compiled.module.words[word_idx];
        let mut max_slot = 0usize;
        for b in word.blocks.iter() {
            for op in b.ops.iter() {
                match op.kind {
                    OpKind::LocalGet { slot, .. } | OpKind::LocalSet { slot, .. } => {
                        max_slot = max_slot.max(slot as usize + 1);
                    }
                    _ => {}
                }
            }
        }
        vec![Slot::int(ir::TY_I64, 0); max_slot]
    }

    fn entry_block(&self, word_idx: usize) -> usize {
        let word = &self.compiled.module.words[word_idx];
        // The verifier rejects a word whose entry block id is unresolvable
        // (EntryBlockNotFound), so this arm is engine-bug-only; routing it to
        // an impossible cursor makes `step` fault loudly (E9158) instead of
        // silently entering block 0 (D-W5: no silent defaults).
        self.block_index(word, word.entry).unwrap_or(usize::MAX)
    }

    fn block_index(&self, word: &ir::Word, id: BlockId) -> Option<usize> {
        word.blocks.iter().position(|b| b.id == id)
    }
}

enum StepResult {
    Continue(Cursor),
    Done,
    Trap(TrapCode, Span),
    /// A contract failure (trap 20, never elidable) carrying the §Q6 detail.
    Contract(Span, TrapDetail),
    Exit(i64),
    /// A REPL-layer device diagnostic (E9152) — the peripheral rejected the
    /// access (unbound aperture), with the aperture id when known.
    Device(u32, Span, Option<u16>),
    Fault(Fault),
}

fn bits_to_width(bits: u16) -> u8 {
    core::cmp::max(1, bits / 8) as u8
}

// ---------------------------------------------------------------------------
// L2a checked pops (DEC-23): every pop names the value kind it expects; a
// mismatch is the contract trap with detail. There is no coercing pop left in
// the engine — the raw-slot accessors on `Value` are display/builtin-surface
// only (the grep gate pins this).
// ---------------------------------------------------------------------------

/// Why a checked pop failed: a real underflow (trap 10) or a kind mismatch
/// (trap 20 with detail).
enum PopFault {
    Underflow,
    Kind(Slot),
}

fn pop_checked(state: &mut ExecState, want: ValKind) -> Result<Slot, PopFault> {
    let s = state.stack.pop().ok_or(PopFault::Underflow)?;
    if s.val.kind() != want {
        return Err(PopFault::Kind(s));
    }
    Ok(s)
}

/// A pop that accepts any kind — depth is still checked (underflow → trap 10).
fn pop_any(state: &mut ExecState) -> Result<Slot, PopFault> {
    state.stack.pop().ok_or(PopFault::Underflow)
}

impl<'c> Engine<'c> {
    /// Convert a pop failure into the step result: underflow keeps the frozen
    /// stack-error numeric (trap 10); a kind mismatch is trap 20 with the
    /// op's span and the §Q6 detail (which word, declared what, actual cell).
    fn pop_fault(&self, f: PopFault, span: Span, word: &ir::Word, want: ValKind) -> StepResult {
        match f {
            PopFault::Underflow => StepResult::Trap(TrapCode::StackOverflow, span),
            PopFault::Kind(s) => StepResult::Contract(
                span,
                TrapDetail {
                    kind: DetailKind::SigMismatch,
                    word: word.name.as_bytes().to_vec(),
                    declared: want.name().to_vec(),
                    actual: vec![cell_of(word, s)],
                },
            ),
        }
    }

    /// Checked pointer pop for the load/store/mmio helpers.
    fn pop_ptr_or(
        &self,
        state: &mut ExecState,
        span: Span,
        word: &ir::Word,
    ) -> Result<u64, StepResult> {
        match pop_checked(state, ValKind::Ptr) {
            Ok(s) => match s.val {
                Value::Ptr(p) => Ok(p),
                _ => unreachable!("pop_checked(Ptr) returned a non-pointer"),
            },
            Err(f) => Err(self.pop_fault(f, span, word, ValKind::Ptr)),
        }
    }

    /// Checked integer pop for the load/store/mmio helpers.
    fn pop_int_or(
        &self,
        state: &mut ExecState,
        span: Span,
        word: &ir::Word,
    ) -> Result<i64, StepResult> {
        match pop_checked(state, ValKind::Int) {
            Ok(s) => match s.val {
                Value::Int(v) => Ok(v),
                _ => unreachable!("pop_checked(Int) returned a non-int"),
            },
            Err(f) => Err(self.pop_fault(f, span, word, ValKind::Int)),
        }
    }

    /// L2b (DEC-23): at a block entry, the runtime top-of-stack types must
    /// equal the block's checked `entry_stack` (types + min depth). Ids are
    /// consistent at every call site of this helper: branch targets live in
    /// the executing word, and a callee's inputs were just retagged into its
    /// table. A mismatch is unreachable from verified IR (§Q7) — an engine
    /// bug, surfaced as E9158 (page-fail), never a student-facing trap.
    fn check_block_entry(
        &self,
        word_idx: usize,
        block_idx: usize,
        state: &ExecState,
        span: Span,
    ) -> Result<(), StepResult> {
        let word = &self.compiled.module.words[word_idx];
        let Some(block) = word.blocks.get(block_idx) else {
            // A bad cursor faults in `step` immediately after; nothing extra
            // to check here.
            return Ok(());
        };
        let n = block.entry_stack.len();
        if state.stack.len() < n {
            return Err(StepResult::Fault(Fault::internal(
                span,
                b"block entry below its checked entry-stack depth (engine bug; see the plan, Q7)",
            )));
        }
        let base = state.stack.len() - n;
        for (i, want) in block.entry_stack.iter().enumerate() {
            if state.stack[base + i].ty != *want {
                return Err(StepResult::Fault(Fault::internal(
                    span,
                    b"block-entry stack types diverge from the checked IR (engine bug; see the plan, Q7)",
                )));
            }
        }
        Ok(())
    }
}

/// One stack cell for a `TrapDetail`/debug payload, rendered through the
/// word's own type table (I-8: engine-derived, never reconstructed).
fn cell_of(word: &ir::Word, s: Slot) -> StackCell {
    StackCell {
        ty_name: type_name_of(word, s.ty),
        value_hex: s.val.hex(),
    }
}

/// The atom name of a word-local `TypeId` (or `?` — never a guess). Public:
/// the debug projection renders through the same name resolution (I-11).
pub fn type_name_of(word: &ir::Word, ty: ir::TypeId) -> Vec<u8> {
    word.types
        .get(ty.0 as usize)
        .map(|a| a.as_bytes().to_vec())
        .unwrap_or_else(|| b"?".to_vec())
}

/// The word's declared signature as canonical `( in -- out )` text (the §5.3
/// `declared` field; the same shape `parse_word_sig` accepts).
pub fn sig_text_of(word: &ir::Word) -> Vec<u8> {
    let mut out = alloc::vec::Vec::new();
    out.extend_from_slice(b"( ");
    for i in 0..word.sig.in_len as usize {
        if i != 0 {
            out.push(b' ');
        }
        out.extend_from_slice(&type_name_of(word, word.sig.inputs[i]));
    }
    out.extend_from_slice(b" --");
    if word.sig.out_len > 0 {
        out.push(b' ');
    }
    for i in 0..word.sig.out_len as usize {
        if i != 0 {
            out.push(b' ');
        }
        out.extend_from_slice(&type_name_of(word, word.sig.outputs[i]));
    }
    out.extend_from_slice(b" )");
    out
}

/// The top `cap` stack cells, bottom-first within the window (for a
/// `TrapDetail.actual`).
fn top_cells(state: &ExecState, word: &ir::Word, cap: usize) -> Vec<StackCell> {
    let start = state.stack.len().saturating_sub(cap);
    state.stack[start..]
        .iter()
        .map(|&s| cell_of(word, s))
        .collect()
}
