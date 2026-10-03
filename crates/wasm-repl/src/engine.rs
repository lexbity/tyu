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
use crate::value::{Slot, Value};

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
    /// bytes (the `testio` surface), `exit_stack` the data stack at exit.
    Ok { out: Vec<u8>, exit_stack: Vec<Slot> },
    /// A trap with the frozen numeric `TrapCode` + the faulting op's span.
    /// Output already emitted stays emitted (§Q3 DEC-6).
    Trap {
        code: TrapCode,
        span: Span,
        out: Vec<u8>,
    },
    /// `testio.exit ( i64 -- )` — the module run terminated with a code
    /// (`0` = pass); the session survives.
    Exited { code: i64, out: Vec<u8> },
    /// Fuel exhausted (E9150 surface): deterministic, resumable.
    Fuel { out: Vec<u8> },
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
}

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
    /// Interned `ConstStr` addresses: `(span.start, addr)`.
    strings: Vec<(u32, u64)>,
    /// Resource-place storage: `(place bytes, addr)`.
    resources: Vec<(Vec<u8>, u64)>,
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
            strings,
            resources: Vec::new(),
        }
    }

    /// A state seeded with the session's persistent stack + heap for one
    /// phrase run (the session holds the committed baseline; §7.3).
    pub fn in_session(
        stack: Vec<Slot>,
        heap: RegionHeap,
        fuel: Fuel,
        elide_guards: bool,
    ) -> ExecState {
        ExecState {
            stack,
            frames: Vec::new(),
            heap,
            out: Vec::new(),
            fuel,
            journal: Journal::new(),
            elide_guards,
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
    /// first-access order — deterministic for a given run, §Q6).
    pub fn resource_addr(&mut self, place: &[u8]) -> u64 {
        if let Some((_, a)) = self.resources.iter().find(|(p, _)| p == place) {
            return *a;
        }
        let addr = self.heap.intern(&[]).unwrap_or(0);
        self.resources.push((place.to_vec(), addr));
        addr
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
            return Outcome::Trap {
                code: TrapCode::Unreachable,
                span: Span::UNKNOWN,
                out: core::mem::take(&mut state.out),
            };
        };
        let DictTarget::Word(word_idx) = target else {
            return Outcome::Trap {
                code: TrapCode::Unreachable,
                span: Span::UNKNOWN,
                out: core::mem::take(&mut state.out),
            };
        };
        if !self.bound_ok(word_idx, state) {
            return Outcome::Trap {
                code: TrapCode::StackOverflow,
                span: Span::UNKNOWN,
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

    fn drive(&self, cursor: Cursor, state: &mut ExecState, periph: &mut dyn Peripheral) -> Outcome {
        let mut cur = cursor;
        loop {
            if !state.fuel.take() {
                return Outcome::Fuel {
                    out: core::mem::take(&mut state.out),
                };
            }
            match self.step(cur, state, periph) {
                StepResult::Continue(next) => cur = next,
                StepResult::Done => {
                    return Outcome::Ok {
                        out: core::mem::take(&mut state.out),
                        exit_stack: state.stack.clone(),
                    };
                }
                StepResult::Trap(code, span) => {
                    return Outcome::Trap {
                        code,
                        span,
                        out: core::mem::take(&mut state.out),
                    };
                }
                StepResult::Exit(code) => {
                    return Outcome::Exited {
                        code,
                        out: core::mem::take(&mut state.out),
                    };
                }
                StepResult::Fault => {
                    return Outcome::Trap {
                        code: TrapCode::Unreachable,
                        span: Span::UNKNOWN,
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
            return StepResult::Fault;
        };
        let Some(op) = block.ops.get(cur.op) else {
            return StepResult::Fault;
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
                None => StepResult::Fault,
            },
            OpKind::AddrOf {
                place,
                mutable: _,
                base,
            } => match base {
                ir::AddrOfBase::Runtime => {
                    let addr = state.resource_addr(place.as_bytes());
                    state.stack.push(Slot::ptr(ir::TY_PTR_MUT, addr));
                    StepResult::Continue(next_op)
                }
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
                Err(()) => StepResult::Fault,
            },
            OpKind::TaskSpawn { .. } => StepResult::Fault,
            OpKind::PtrAddConst { offset, .. } => match pop_ptr(state) {
                Ok(p) => {
                    state
                        .stack
                        .push(Slot::ptr(ir::TY_PTR_MUT, p.wrapping_add(u64::from(offset))));
                    StepResult::Continue(next_op)
                }
                Err(t) => StepResult::Trap(t, op.span),
            },
            OpKind::PtrAddIndex { scale, .. } => match (pop_int(state), pop_ptr(state)) {
                (Ok(idx), Ok(base)) => {
                    let addr = base.wrapping_add((idx as u64).wrapping_mul(u64::from(scale)));
                    state.stack.push(Slot::ptr(ir::TY_PTR_MUT, addr));
                    StepResult::Continue(next_op)
                }
                (Err(t), _) | (_, Err(t)) => StepResult::Trap(t, op.span),
            },
            OpKind::Dup { .. } => {
                // Single-slot reference/value copy (native `emit_dup`).
                let top = *state.stack.last().unwrap_or(&Slot::int(ir::TY_I64, 0));
                state.stack.push(top);
                StepResult::Continue(next_op)
            }
            OpKind::Drop { .. } => {
                let _ = state.stack.pop();
                StepResult::Continue(next_op)
            }
            OpKind::Swap { .. } => {
                let n = state.stack.len();
                if n >= 2 {
                    state.stack.swap(n - 1, n - 2);
                }
                StepResult::Continue(next_op)
            }
            OpKind::AddI64 => match (pop_int(state), pop_int(state)) {
                (Ok(b), Ok(a)) => {
                    // G53 red-path instrument: under `wasm-selftest` this arm
                    // subtracts so the execution differential MUST fail.
                    let r = if selftest_corrupt_add() {
                        a.wrapping_sub(b)
                    } else {
                        a.wrapping_add(b)
                    };
                    state.stack.push(Slot::int(ir::TY_I64, r));
                    StepResult::Continue(next_op)
                }
                (Err(t), _) | (_, Err(t)) => StepResult::Trap(t, op.span),
            },
            OpKind::SubI64 => match (pop_int(state), pop_int(state)) {
                (Ok(b), Ok(a)) => {
                    state.stack.push(Slot::int(ir::TY_I64, a.wrapping_sub(b)));
                    StepResult::Continue(next_op)
                }
                (Err(t), _) | (_, Err(t)) => StepResult::Trap(t, op.span),
            },
            OpKind::MulI64 => match (pop_int(state), pop_int(state)) {
                (Ok(b), Ok(a)) => {
                    state.stack.push(Slot::int(ir::TY_I64, a.wrapping_mul(b)));
                    StepResult::Continue(next_op)
                }
                (Err(t), _) | (_, Err(t)) => StepResult::Trap(t, op.span),
            },
            OpKind::Cmp { out, kind } => match (pop_int(state), pop_int(state)) {
                (Ok(b), Ok(a)) => {
                    let r = match kind {
                        ir::CmpKind::Lt => a < b,
                        ir::CmpKind::Le => a <= b,
                        ir::CmpKind::Gt => a > b,
                        ir::CmpKind::Ge => a >= b,
                        ir::CmpKind::Eq => a == b,
                        ir::CmpKind::Ne => a != b,
                    };
                    state.stack.push(Slot {
                        ty: out,
                        val: Value::Bool(r),
                    });
                    StepResult::Continue(next_op)
                }
                (Err(t), _) | (_, Err(t)) => StepResult::Trap(t, op.span),
            },
            OpKind::AndBool => match (pop_truthy(state), pop_truthy(state)) {
                (Ok(b), Ok(a)) => {
                    state.stack.push(Slot::bool(a && b));
                    StepResult::Continue(next_op)
                }
                (Err(t), _) | (_, Err(t)) => StepResult::Trap(t, op.span),
            },
            OpKind::OrBool => match (pop_truthy(state), pop_truthy(state)) {
                (Ok(b), Ok(a)) => {
                    state.stack.push(Slot::bool(a || b));
                    StepResult::Continue(next_op)
                }
                (Err(t), _) | (_, Err(t)) => StepResult::Trap(t, op.span),
            },
            OpKind::NotBool => match pop_truthy(state) {
                Ok(a) => {
                    state.stack.push(Slot::bool(!a));
                    StepResult::Continue(next_op)
                }
                Err(t) => StepResult::Trap(t, op.span),
            },
            OpKind::InterruptDisable | OpKind::InterruptEnable => StepResult::Fault,
            OpKind::LocalSet { slot, ty: _ } => match pop_slot(state) {
                Ok(v) => {
                    if let Some(f) = state.frames.last_mut() {
                        let idx = slot as usize;
                        if f.locals.len() <= idx {
                            f.locals.resize(idx + 1, Slot::int(ir::TY_I64, 0));
                        }
                        f.locals[idx] = v;
                    }
                    StepResult::Continue(next_op)
                }
                Err(t) => StepResult::Trap(t, op.span),
            },
            OpKind::LocalGet { slot, ty } => {
                let v = state
                    .frames
                    .last()
                    .and_then(|f| f.locals.get(slot as usize))
                    .copied()
                    .unwrap_or(Slot::int(ty, 0));
                state.stack.push(v);
                StepResult::Continue(next_op)
            }
            OpKind::Cast { from, to } => match pop_slot(state) {
                Ok(v) => match self.step_cast(word, from, to, v) {
                    Ok(out_slot) => {
                        state.stack.push(out_slot);
                        StepResult::Continue(next_op)
                    }
                    Err(()) => StepResult::Trap(TrapCode::SubtypeFail, op.span),
                },
                Err(t) => StepResult::Trap(t, op.span),
            },
            OpKind::Bitcast { to, .. } => match pop_slot(state) {
                Ok(v) => {
                    state.stack.push(Slot { ty: to, val: v.val });
                    StepResult::Continue(next_op)
                }
                Err(t) => StepResult::Trap(t, op.span),
            },
            OpKind::Call { name, sig, .. } => match self.compiled.dict_lookup(name.as_bytes()) {
                Some(DictTarget::Builtin(b)) => match self.call_builtin(b, state) {
                    Ok(()) => StepResult::Continue(next_op),
                    Err(BuiltinSignal::Exit(code)) => StepResult::Exit(code),
                    Err(BuiltinSignal::Trap(code)) => StepResult::Trap(code, op.span),
                },
                Some(DictTarget::Word(callee)) => {
                    // L2: stack depth ≥ sig.in_len (never elidable).
                    if state.stack.len() < sig.in_len as usize {
                        return StepResult::Trap(TrapCode::StackOverflow, op.span);
                    }
                    if state.frames.len() + 1 >= CALL_DEPTH_CAP {
                        return StepResult::Trap(TrapCode::StackOverflow, op.span);
                    }
                    if !self.bound_ok(callee, state) {
                        return StepResult::Trap(TrapCode::StackOverflow, op.span);
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
                    StepResult::Continue(Cursor {
                        word: callee,
                        block: self.entry_block(callee),
                        op: 0,
                    })
                }
                None => StepResult::Fault,
            },
            OpKind::Load { ty } => match self.step_load(word, ty, state) {
                Ok(()) => StepResult::Continue(next_op),
                Err(()) => StepResult::Trap(TrapCode::AssertFail, op.span),
            },
            OpKind::Store { ty } => match self.step_store(word, ty, state) {
                Ok(()) => StepResult::Continue(next_op),
                Err(()) => StepResult::Trap(TrapCode::AssertFail, op.span),
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
            OpKind::TrapIfFalse { code } => {
                let v = state.stack.pop();
                if state.elide_guards {
                    // L1 elision (§Q4): the emitted guard is skipped — the
                    // bool is consumed, control falls through. L2/L3
                    // interpreter invariants are unaffected.
                    StepResult::Continue(next_op)
                } else {
                    match v {
                        Some(slot) if slot.val.truthy() => StepResult::Continue(next_op),
                        _ => StepResult::Trap(code, op.span),
                    }
                }
            }
            OpKind::Br { target } => match self.block_index(word, target) {
                Some(b) => StepResult::Continue(Cursor {
                    word: cur.word,
                    block: b,
                    op: 0,
                }),
                None => StepResult::Fault,
            },
            OpKind::BrIf { then_tgt, else_tgt } => {
                let truthy = state.stack.pop().map(|s| s.val.truthy()).unwrap_or(false);
                let target = if truthy { then_tgt } else { else_tgt };
                match self.block_index(word, target) {
                    Some(b) => StepResult::Continue(Cursor {
                        word: cur.word,
                        block: b,
                        op: 0,
                    }),
                    None => StepResult::Fault,
                }
            }
            OpKind::Ret => match state.frames.pop().and_then(|f| f.ret) {
                Some(ret) => StepResult::Continue(ret),
                None => StepResult::Done,
            },
        }
    }

    fn step_scoped_enter(
        &self,
        word: &ir::Word,
        ty: ir::TypeId,
        len: u32,
        state: &mut ExecState,
    ) -> Result<(), ()> {
        match word.type_classes.get(ty.0 as usize).copied() {
            // Slice: a `{ptr,len}` descriptor whose address is pushed (native
            // stores it in the frame's scoped area; the heap is the concrete
            // arena here, §6.7).
            Some(ir::TypeClass::Slice) => {
                let base = state.stack.last().map(|s| s.val.as_u64()).unwrap_or(0);
                let addr = state
                    .heap
                    .scoped_enter(base, u64::from(len))
                    .map_err(|_| ())?;
                state.stack.push(Slot::ptr(ty, addr));
                // Journal the descriptor allocation so a rollback drops its
                // live range (§7.3).
                state.journal.push(Effect::ScopedEnter { addr });
                Ok(())
            }
            Some(ir::TypeClass::RegionRef | ir::TypeClass::RegionRefMut) => {
                let top = *state.stack.last().unwrap_or(&Slot::int(ir::TY_I64, 0));
                state.stack.push(top);
                Ok(())
            }
            _ => Err(()),
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

    fn step_load(&self, word: &ir::Word, ty: ir::TypeId, state: &mut ExecState) -> Result<(), ()> {
        let (bits, signed) =
            ir::storage_bits_signed(word, ty, self.spec.word_bits as u16).ok_or(())?;
        let width = bits_to_width(bits);
        let addr = pop_ptr(state).map_err(|_| ())?;
        let raw = state.heap.load(addr, width).map_err(|_| ())?;
        let v = match (width, signed) {
            (1, true) => (raw as u8 as i8) as i64,
            (1, false) => (raw as u8) as i64,
            (2, true) => (raw as u16 as i16) as i64,
            (2, false) => (raw as u16) as i64,
            (4, true) => (raw as u32 as i32) as i64,
            (4, false) => raw as u32 as i64,
            (8, _) => raw as i64,
            _ => return Err(()),
        };
        state.stack.push(Slot::int(ty, v));
        Ok(())
    }

    fn step_store(&self, word: &ir::Word, ty: ir::TypeId, state: &mut ExecState) -> Result<(), ()> {
        let (bits, _) = ir::storage_bits_signed(word, ty, self.spec.word_bits as u16).ok_or(())?;
        let width = bits_to_width(bits);
        let v = pop_int(state).map_err(|_| ())?;
        let addr = pop_ptr(state).map_err(|_| ())?;
        let old = state.heap.store(addr, width, v as u64).map_err(|_| ())?;
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
        let width = self.mmio_width(word, ty).ok_or(StepResult::Fault)?;
        let addr = pop_ptr(state).map_err(|t| StepResult::Trap(t, span))?;
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
        let width = self.mmio_width(word, ty).ok_or(StepResult::Fault)?;
        let v = pop_int(state).map_err(|t| StepResult::Trap(t, span))?;
        let addr = pop_ptr(state).map_err(|t| StepResult::Trap(t, span))?;
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
            return Err(StepResult::Fault);
        };
        let span = op.span;
        let width = self.mmio_width(word, reg_ty).ok_or(StepResult::Fault)?;
        let addr = pop_ptr(state).map_err(|t| StepResult::Trap(t, span))?;
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
            return Err(StepResult::Fault);
        };
        let span = op.span;
        let width = self.mmio_width(word, reg_ty).ok_or(StepResult::Fault)?;
        let field = pop_int(state).map_err(|t| StepResult::Trap(t, span))?;
        let addr = pop_ptr(state).map_err(|t| StepResult::Trap(t, span))?;
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
        self.block_index(word, word.entry).unwrap_or(0)
    }

    fn block_index(&self, word: &ir::Word, id: BlockId) -> Option<usize> {
        word.blocks.iter().position(|b| b.id == id)
    }
}

enum StepResult {
    Continue(Cursor),
    Done,
    Trap(TrapCode, Span),
    Exit(i64),
    /// A REPL-layer device diagnostic (E9152) — the peripheral rejected the
    /// access (unbound aperture), with the aperture id when known.
    Device(u32, Span, Option<u16>),
    Fault,
}

fn bits_to_width(bits: u16) -> u8 {
    core::cmp::max(1, bits / 8) as u8
}

fn pop_int(state: &mut ExecState) -> Result<i64, TrapCode> {
    match state.stack.pop() {
        Some(s) => Ok(s.val.as_i64()),
        None => Err(TrapCode::StackOverflow),
    }
}

fn pop_ptr(state: &mut ExecState) -> Result<u64, TrapCode> {
    match state.stack.pop() {
        Some(s) => Ok(s.val.as_u64()),
        None => Err(TrapCode::StackOverflow),
    }
}

fn pop_slot(state: &mut ExecState) -> Result<Slot, TrapCode> {
    state.stack.pop().ok_or(TrapCode::StackOverflow)
}

fn pop_truthy(state: &mut ExecState) -> Result<bool, TrapCode> {
    match state.stack.pop() {
        Some(s) => Ok(s.val.truthy()),
        None => Err(TrapCode::StackOverflow),
    }
}
