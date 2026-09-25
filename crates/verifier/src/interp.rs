//! Abstract interpretation over the IR (static-verification.md §7.2, slice
//! P5): the in-tree interval engine.
//!
//! ```text
//! Interval = ⊥ | [lo, hi] | ⊤        (standard interval lattice)
//! Origin   = Arg(i) | Local(i) | Computed | ⊤  (identity lattice — P6's E3312)
//! State    = { stack: slot → Interval×Origin, locals: id → Interval×Origin }
//! ```
//!
//! [`State::step`] is the normative transfer table (§7.2) over the IR ops the
//! lowering emits, with the arithmetic rules of [`crate::interval`] (checked
//! bounds, overflow → `⊤`, sound versus wrapping). **Memory is not modeled**
//! (Q4): `Load`/`MmioVol*` yield `⊤` unconditionally; `Call` yields `⊤`
//! outputs. `BrIf` joins by hull union; conditions do not refine branches (no
//! path sensitivity in v1).
//!
//! Two consumers share this one transfer table:
//! - [`Linear`] — the per-block states the *lowering* maintains incrementally
//!   (the emission decision for each obligation site: the emit-site is in the
//!   same function as the decision, per P4's discipline);
//! - [`run_cfg`] / [`discharge_word`] — the full block-CFG worklist engine
//!   with back-edge widening (FR-12: loop-carried slots reach `⊤` in ≤ 2
//!   passes), exercised by the differential soundness harness (FR-20).

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use ir::{BlockId, OpKind, TypeId};

use crate::interval::{Interval, Tri};

pub use crate::interval::{bool_iv, eval_in_range, tri_and, tri_cmp, tri_from_bool_iv, tri_not, tri_or};

/// Max abstract stack/ local depth (mirrors the IR's own slot caps; the
/// abstract state saturates, never panics).
pub const MAX_SLOTS: usize = 64;

/// The identity lattice (P6's E3312 substrate; carried now, exercised then).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    /// The value derives from word input slot `i`.
    Arg(u32),
    /// The value derives from local slot `i`.
    Local(u32),
    /// The value was computed by an op (not an identity-preserving move).
    Computed,
    /// Unknown / top identity.
    Top,
}

/// One abstract stack/local slot: `(Interval × Origin)` (Q4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Slot {
    pub iv: Interval,
    pub origin: Origin,
}

impl Slot {
    pub fn top() -> Slot {
        Slot {
            iv: Interval::TOP,
            origin: Origin::Top,
        }
    }

    pub fn computed(iv: Interval) -> Slot {
        Slot {
            iv,
            origin: Origin::Computed,
        }
    }
}

/// The abstract state: data-stack vector + locals map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct State {
    pub stack: Vec<Slot>,
    pub locals: Vec<Slot>,
}

/// The subtype range of an IR type id, when that id is a subtype declaration.
/// Supplied by the lowering (which owns the subtype table); the engine stays
/// compiler-coupled only through this lookup.
pub type SubtypeRange<'s> = dyn Fn(TypeId) -> Option<(i64, i64)> + 's;

impl State {
    /// A fresh state with `locals_cap` (pre-filled to `⊤`) and an empty
    /// stack.
    pub fn fresh(locals_cap: usize) -> State {
        let cap = locals_cap.clamp(0, MAX_SLOTS);
        let mut locals = Vec::with_capacity(cap);
        for _ in 0..cap {
            locals.push(Slot::top());
        }
        State {
            stack: Vec::new(),
            locals,
        }
    }

    /// The callee-entry state: the word inputs on the (abstract) stack, all
    /// `⊤` (`Arg(i)` origins — the callee cannot know its callers, Q4).
    pub fn callee_entry(inputs: usize, locals_cap: usize) -> State {
        let mut s = State::fresh(locals_cap);
        for i in 0..inputs {
            s.stack.push(Slot {
                iv: Interval::TOP,
                origin: Origin::Arg(i as u32),
            });
        }
        s
    }

    /// The top-of-stack interval as the region of obligation evaluation.
    pub fn top_interval(&self) -> Interval {
        self.stack.last().map_or(Interval::BOTTOM, |s| s.iv)
    }

    /// The top `n` stack intervals, bottom-most of the top run first
    /// (`out.i` maps to the i-th of these).
    pub fn top_n_intervals(&self, n: usize) -> Vec<Interval> {
        let len = self.stack.len();
        let start = len.saturating_sub(n);
        (start..len).map(|i| self.stack[i].iv).collect()
    }

    /// Truncate the abstract stack to `len` (drop `len` and beyond no-ops).
    pub fn truncate_stack(&mut self, len: usize) {
        if self.stack.len() > len {
            self.stack.truncate(len);
        }
    }

    /// A contract-predicate call (slice P6, Q6): pop `in_len` argument slots
    /// and push them back **unchanged**, then push the predicate's extra
    /// outputs (the verdict) as computed `⊤`. Soundness comes from the
    /// purity check (E3313): a predicate was verified store/spawn/effect-free
    /// at its own declaration, so the subject values it returns in the same
    /// positions are the same values (the origins pass through). This is the
    /// plan's "needs-discharged callees MAY pass through" for the identity
    /// lattice — without it, any named-predicate reference would fail E3312.
    pub fn predicate_call(&mut self, in_len: usize, out_len: usize) {
        let mut args = Vec::with_capacity(in_len);
        for _ in 0..in_len {
            args.push(self.pop_val());
        }
        // args is top-first; re-push in original (bottom-first) order.
        for a in args.iter().rev() {
            self.push(*a);
        }
        let extra = out_len.saturating_sub(in_len);
        for _ in 0..extra {
            self.push(Slot::computed(Interval::TOP));
        }
    }

    fn push(&mut self, slot: Slot) {
        if self.stack.len() >= MAX_SLOTS {
            // Abstract saturation: never panic; soundness is unaffected (a
            // full abstract stack means the concrete stack is deep, and the
            // engine resolves those slots conservatively by their last value).
            self.stack.truncate(MAX_SLOTS - 1);
        }
        self.stack.push(slot);
    }

    fn push_top(&mut self) {
        self.push(Slot::top());
    }

    fn pop_val(&mut self) -> Slot {
        self.stack.pop().unwrap_or(Slot::top())
    }

    fn pop2_val(&mut self) -> (Slot, Slot) {
        // second = top, first = below (matches the runtime `rax op rcx`
        // operand order of `emit_binop`).
        let second = self.pop_val();
        let first = self.pop_val();
        (first, second)
    }

    fn binop(&mut self, f: fn(Interval, Interval) -> Interval) {
        let (first, second) = self.pop2_val();
        self.push(Slot::computed(f(first.iv, second.iv)));
    }

    /// The normative transfer function (§7.2 table). Soundness: every op
    /// either computes a conservative interval in [`crate::interval`], or
    /// widens to `⊤` (memory, calls, addresses, unknown types).
    pub fn step(&mut self, op: &OpKind, sr: &SubtypeRange<'_>) {
        match op {
            OpKind::ConstI64(v) => self.push(Slot::computed(Interval::const_val(*v))),
            OpKind::ConstBool(b) => self.push(Slot::computed(Interval::const_val(if *b { 1 } else { 0 }))),
            OpKind::ConstStr(_) => self.push_top(),
            OpKind::AddrOf { .. } | OpKind::MmioPlace { .. } | OpKind::ScopedEnter { .. } => {
                self.push_top()
            }
            OpKind::TaskSpawn { .. } => self.push_top(),
            OpKind::PtrAddConst { .. } => {
                let _ = self.pop_val();
                self.push_top();
            }
            OpKind::PtrAddIndex { .. } => {
                let _ = self.pop2_val();
                self.push_top();
            }
            OpKind::Dup { .. } => {
                let v = self.pop_val();
                self.push(v);
                self.push(v);
            }
            OpKind::Drop { .. } => {
                let _ = self.pop_val();
            }
            OpKind::Swap { .. } => {
                let (a, b) = self.pop2_val(); // a below, b top
                self.push(b);
                self.push(a);
            }
            OpKind::AddI64 => self.binop(Interval::add),
            OpKind::SubI64 => self.binop(Interval::sub),
            OpKind::MulI64 => self.binop(Interval::mul),
            OpKind::Cmp { kind, .. } => {
                let (a, b) = self.pop2_val();
                let t = tri_cmp(a.iv, b.iv, *kind);
                self.push(Slot::computed(bool_iv(t)));
            }
            OpKind::AndBool => {
                let (a, b) = self.pop2_val();
                let t = tri_and(tri_from_bool_iv(a.iv), tri_from_bool_iv(b.iv));
                self.push(Slot::computed(bool_iv(t)));
            }
            OpKind::OrBool => {
                let (a, b) = self.pop2_val();
                let t = tri_or(tri_from_bool_iv(a.iv), tri_from_bool_iv(b.iv));
                self.push(Slot::computed(bool_iv(t)));
            }
            OpKind::NotBool => {
                let a = self.pop_val();
                let t = tri_not(tri_from_bool_iv(a.iv));
                self.push(Slot::computed(bool_iv(t)));
            }
            OpKind::InterruptDisable | OpKind::InterruptEnable => {}
            OpKind::LocalSet { slot, .. } => {
                let v = self.pop_val();
                let idx = *slot as usize;
                if idx < self.locals.len() {
                    self.locals[idx] = v;
                }
            }
            OpKind::LocalGet { slot, .. } => {
                let idx = *slot as usize;
                let v = self
                    .locals
                    .get(idx)
                    .copied()
                    .unwrap_or(Slot::top());
                self.push(v);
            }
            OpKind::Cast { from: _, to } => {
                let v = self.pop_val();
                // Narrowing cast to a subtype: the abstract successor is the
                // intersection with the target range (§7.2). The *site's own*
                // obligation is evaluated on the PRE-cast value by the caller —
                // the transfer here only feeds downstream tracking.
                let iv = match sr(*to) {
                    Some((lo, hi)) => v.iv.cast_narrow(lo, hi),
                    None => v.iv,
                };
                self.push(Slot {
                    iv,
                    origin: Origin::Computed,
                });
            }
            OpKind::Bitcast { .. } => {
                let v = self.pop_val();
                self.push(v);
            }
            OpKind::Call { sig, .. } => {
                for _ in 0..sig.in_len {
                    let _ = self.pop_val();
                }
                for _ in 0..sig.out_len {
                    self.push(Slot::computed(Interval::TOP));
                }
            }
            OpKind::Load { .. } => {
                let _ = self.pop_val(); // address
                self.push(Slot::computed(Interval::TOP)); // memory not modeled (Q4)
            }
            OpKind::Store { .. } => {
                let _ = self.pop2_val(); // address, value
            }
            OpKind::MmioVolLoad { .. } | OpKind::MmioVolLoadField { .. } => {
                let _ = self.pop_val();
                self.push(Slot::computed(Interval::TOP));
            }
            OpKind::MmioVolStore { .. } | OpKind::MmioVolStoreField { .. } => {
                let _ = self.pop2_val();
            }
            OpKind::TrapIfFalse { .. } => {
                let _ = self.pop_val();
            }
            OpKind::BrIf { .. } => {
                // Control: pops the condition; the CFG engine routes flow.
                let _ = self.pop_val();
            }
            OpKind::Br { .. } | OpKind::Ret => {}
        }
    }
}

/// The in-tree discharge result for an obligation site (slice P5): a
/// *decision input* for [`crate::model::ResolvedVerdict`], never `Assumed`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InTreeVerdict {
    /// `Discharged` only on a lattice proof; `Open` otherwise (fail-closed,
    /// FR-13).
    pub status: crate::verdict::VerdictStatus,
    /// Interval provably outside the target range — recorded
    /// `provably_failing`, check retained.
    pub provably_failing: bool,
    /// Human-readable `no-open` reason (FR-18 quality bar).
    pub reason: Option<String>,
}

impl InTreeVerdict {
    /// The verdict of evaluating an interval against a target range.
    /// Reasons are built by hand (no `alloc::format!` — its unwinding landing
    /// pad breaks the hosted `-nodefaultlibs` link; same discipline as
    /// `verifier::model`).
    pub fn of_range(val: Interval, lo: i64, hi: i64) -> InTreeVerdict {
        match eval_in_range(val, lo, hi) {
            Tri::DefTrue => InTreeVerdict {
                status: crate::verdict::VerdictStatus::Discharged,
                provably_failing: false,
                reason: Some(range_reason(val, lo, hi, true)),
            },
            Tri::DefFalse => InTreeVerdict {
                status: crate::verdict::VerdictStatus::Open,
                provably_failing: true,
                reason: Some(range_reason(val, lo, hi, false)),
            },
            Tri::Top => InTreeVerdict {
                status: crate::verdict::VerdictStatus::Open,
                provably_failing: false,
                reason: Some(range_reason(val, lo, hi, false)),
            },
        }
    }

    /// A plain open verdict without an interval reason (e.g. the callee input
    /// path is always unproven).
    pub fn open(reason: &str) -> InTreeVerdict {
        InTreeVerdict {
            status: crate::verdict::VerdictStatus::Open,
            provably_failing: false,
            reason: Some(reason.to_string()),
        }
    }
}

/// `"value interval X vs target [lo, hi]"` with the three-valued outcome.
fn range_reason(val: Interval, lo: i64, hi: i64, proven: bool) -> String {
    let mut s = String::with_capacity(40);
    s.push_str(if proven { "provably " } else { "" });
    s.push_str("interval ");
    push_iv(&mut s, val);
    s.push_str(" vs target [");
    push_i64(&mut s, lo);
    s.push_str(", ");
    push_i64(&mut s, hi);
    s.push(']');
    s
}

/// Hand-rolled interval token (the `&'static str` shapes stay ASCII-safe).
fn push_iv(s: &mut String, iv: Interval) {
    match iv {
        Interval::Bottom => s.push_str("<bottom>"),
        Interval::Top => s.push_str("<top>"),
        Interval::Range { lo, hi } => {
            s.push('[');
            push_i64(s, lo);
            s.push_str(", ");
            push_i64(s, hi);
            s.push(']');
        }
    }
}

/// Append `v` in decimal (no alloc::format).
fn push_i64(s: &mut String, mut v: i64) {
    if v == i64::MIN {
        s.push_str("-9223372036854775808");
        return;
    }
    if v < 0 {
        s.push('-');
        v = -v;
    }
    let mut buf = [0u8; 20];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    for i in (0..n).rev() {
        s.push(buf[i] as char);
    }
}

// ---------------------------------------------------------------------------
// The per-block linear path (emission-time). One `State` per block id; the
// lowering feeds ops through and seeds/joins/widens at the block
// boundaries it creates (control-flow lowering, control.rs).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Linear {
    blocks: Vec<State>,
}

impl Linear {
    /// Begin a word: block 0 holds the callee-entry state. `blocks < 16`
    /// (the IR's cap); `inputs` seeds the entry stack.
    pub fn new(inputs: usize, locals_cap: usize) -> Linear {
        Linear {
            blocks: vec![State::callee_entry(inputs, locals_cap)],
        }
    }

    /// Transfer an op into block `b` (§7.2). The block's entry state must
    /// already exist (seeded by the control-flow lowering); a missing state
    /// is created fresh (⊤) defensively.
    pub fn step(&mut self, b: BlockId, op: &OpKind, sr: &SubtypeRange<'_>) {
        let idx = b.0 as usize;
        if idx >= self.blocks.len() {
            while self.blocks.len() <= idx {
                self.blocks.push(State::fresh(64));
            }
        }
        self.blocks[idx].step(op, sr);
    }

    /// Slice P6 (Q6): a contract-predicate call's arguments pass through
    /// unchanged (purity was verified at the callee's declaration). See
    /// [`State::predicate_call`].
    pub fn predicate_call(&mut self, b: BlockId, in_len: usize, out_len: usize) {
        let idx = b.0 as usize;
        if idx >= self.blocks.len() {
            while self.blocks.len() <= idx {
                self.blocks.push(State::fresh(64));
            }
        }
        self.blocks[idx].predicate_call(in_len, out_len);
    }

    /// The current folded state of block `b` (never panics: an unseeded
    /// block reads as a fresh ⊤ state).
    pub fn state(&self, b: BlockId) -> &State {
        let idx = b.0 as usize;
        self.blocks.get(idx).expect("interp: block state exists")
    }

    /// The current folded state of block `b`, or a fresh (⊤) fallback when
    /// the block was never seeded.
    pub fn state_or_fresh(&self, b: BlockId) -> State {
        let idx = b.0 as usize;
        self.blocks.get(idx).cloned().unwrap_or_else(|| State::fresh(64))
    }

    /// The top interval of block `b`'s abstract stack.
    pub fn top(&self, b: BlockId) -> Interval {
        self.state(b).top_interval()
    }

    /// The abstract value of local slot `i` in block `b`.
    pub fn local_interval(&self, b: BlockId, i: usize) -> Interval {
        self.state(b)
            .locals
            .get(i)
            .map_or(Interval::TOP, |s| s.iv)
    }

    /// Seed `target` from block `src`, truncated to the branch-entry stack
    /// depth (locals flow across blocks unchanged).
    pub fn seed_from(&mut self, target: BlockId, src: BlockId, stack_len: usize) {
        let idx = target.0 as usize;
        while self.blocks.len() <= idx {
            self.blocks.push(State::fresh(64));
        }
        let mut s = self.state(src).clone();
        s.truncate_stack(stack_len);
        self.blocks[idx] = s;
    }

    /// Seed `target` as the hull join of blocks `a` and `b` (a control-flow
    /// join, §7.2 `BrIf` row) truncated to `stack_len`.
    pub fn seed_join(&mut self, target: BlockId, a: BlockId, b: BlockId, stack_len: usize) {
        let idx = target.0 as usize;
        while self.blocks.len() <= idx {
            self.blocks.push(State::fresh(64));
        }
        let sa = self.state(a);
        let sb = self.state(b);
        self.blocks[idx] = hull_state(sa, sb, stack_len);
    }

    /// Back-edge widening (FR-12/§7.2): for each slot in header, if the
    /// body-end value is a SUBSET of the header value keep the header
    /// (monotone), else widen to `⊤`. Locals likewise. Loop-carried slots
    /// reach `⊤` in ≤ 2 passes.
    pub fn widen_loop(&mut self, header: BlockId, body_end: BlockId, stack_len: usize) {
        let idx = header.0 as usize;
        let h = self.blocks[idx].clone();
        let b = self.state(body_end).clone();
        self.blocks[idx] = widen_state(&h, &b, stack_len);
    }

    /// Seed an exit block from the (possibly widened) header — the loop-exit
    /// path sees the loop-carried values.
    pub fn seed_exit(&mut self, exit: BlockId, src: BlockId, stack_len: usize) {
        self.seed_from(exit, src, stack_len);
    }
}

/// The two-state hull join: per-index `a ∨ b` (stack positions up to
/// `stack_len`; locals everywhere; stack beyond `stack_len` dropped).
fn hull_state(a: &State, b: &State, stack_len: usize) -> State {
    let n = core::cmp::min(stack_len, core::cmp::min(a.stack.len(), b.stack.len()));
    let mut stack = Vec::with_capacity(n);
    for i in 0..n {
        let x = a.stack[i];
        let y = b.stack[i];
        stack.push(Slot {
            iv: x.iv.join(y.iv),
            origin: if x.origin == y.origin { x.origin } else { Origin::Computed },
        });
    }
    let locals_cap = core::cmp::max(a.locals.len(), b.locals.len());
    let mut locals = Vec::with_capacity(locals_cap);
    for i in 0..locals_cap {
        let x = a.locals.get(i).copied().unwrap_or(Slot::top());
        let y = b.locals.get(i).copied().unwrap_or(Slot::top());
        locals.push(Slot {
            iv: x.iv.join(y.iv),
            origin: if x.origin == y.origin { x.origin } else { Origin::Computed },
        });
    }
    State { stack, locals }
}

/// The back-edge widening: `v ∇ old = old if new ⊑ old, else ⊤` (§7.2).
fn widen_state(header: &State, body_end: &State, stack_len: usize) -> State {
    let n = core::cmp::min(stack_len, core::cmp::min(header.stack.len(), body_end.stack.len()));
    let mut stack = Vec::with_capacity(n);
    for i in 0..n {
        let old = header.stack[i];
        let new = body_end.stack[i];
        stack.push(Slot {
            iv: if new.iv.subset_of(old.iv) { old.iv } else { Interval::TOP },
            origin: if old.origin == Origin::Top || old.origin == new.origin {
                old.origin
            } else {
                Origin::Computed
            },
        });
    }
    let locals_cap = core::cmp::max(header.locals.len(), body_end.locals.len());
    let mut locals = Vec::with_capacity(locals_cap);
    for i in 0..locals_cap {
        let old = header.locals.get(i).copied().unwrap_or(Slot::top());
        let new = body_end.locals.get(i).copied().unwrap_or(Slot::top());
        locals.push(Slot {
            iv: if new.iv.subset_of(old.iv) { old.iv } else { Interval::TOP },
            origin: if old.origin == Origin::Top || old.origin == new.origin {
                old.origin
            } else {
                Origin::Computed
            },
        });
    }
    State { stack, locals }
}

// ---------------------------------------------------------------------------
// The full CFG engine (`run_cfg`): worklist to fixpoint, back-edge widening
// (FR-12). Used by the differential soundness harness (and the semantics
// integration tests); the emission-time decisions use [`Linear`] (identical
// transfer table).
// ---------------------------------------------------------------------------

/// The result of a full CFG fixpoint: the per-block entry states.
pub struct CfResult {
    pub states: Vec<State>,
}

/// Worklist fixpoint over the word CFG (NFR-7: ≤ 16 blocks, single order-
/// independent pass with per-back-edge widening). Starting from block 0's
/// callee-entry state, propagate through `Br`/`BrIf`. A target block already
/// visited is a **back-edge**: its state merges by the ∇ widening (§7.2/FR-12:
/// keep `old` when `new ⊑ old`, else `⊤`), so loop-carried slots reach `⊤` in
/// ≤ 2 visits; a first visit hull-joins (the target may have fallen through
/// several entries). Terminates by construction.
pub fn run_cfg(word: &ir::Word, sr: &SubtypeRange<'_>) -> CfResult {
    let nblocks = word.blocks.len().max(1);
    let mut states: Vec<State> = vec![State::callee_entry(word.sig.in_len as usize, 64)];
    while states.len() < nblocks {
        states.push(State::fresh(64));
    }
    let mut worklist: Vec<BlockId> = vec![BlockId(0)];
    let mut visits = vec![0u32; nblocks];
    let mut budget = nblocks.saturating_mul(4).saturating_add(2);

    while let Some(bid) = worklist.pop() {
        if budget == 0 {
            break; // conservative: remaining blocks stay at their last state
        }
        budget -= 1;
        let idx = bid.0 as usize;
        let count = visits[idx];
        visits[idx] = count.saturating_add(1);
        let Some(block) = word.blocks.get(idx) else {
            continue;
        };
        let mut flow = states[idx].clone();
        for op in block.ops.iter() {
            flow.step(&op.kind, sr);
        }
        // Propagate to block terminators' successors (Ret/Load-flows are
        // intra-block; Br/BrIf route).
        for tgt in block_successors_from(&block) {
            let ti = tgt.0 as usize;
            if visits[ti] > 0 {
                // Back-edge (FR-12 widening: v ∇ old = old if new ⊑ old else ⊤).
                states[ti] = widen_state(&states[ti], &flow, word.sig.out_len as usize);
                worklist.push(tgt);
            } else {
                // First arrival: hull-join with any prior (⊤) placeholder.
                states[ti] = merge_first_visit(&states[ti], &flow);
                worklist.push(tgt);
            }
        }
    }
    CfResult { states }
}

fn block_successors_from(block: &ir::Block) -> Vec<BlockId> {
    match block.ops.iter().last().map(|op| &op.kind) {
        Some(OpKind::Br { target }) => vec![*target],
        Some(OpKind::BrIf {
            then_tgt,
            else_tgt,
        }) => vec![*then_tgt, *else_tgt],
        _ => Vec::new(),
    }
}

/// Join an incoming flow into a target block's preexisting placeHolder on its
/// first arrival: `old ∨ incoming` per slot (a first visit may still join two
/// entries of a diamond whose far branch was seeded first).
fn merge_first_visit(old: &State, incoming: &State) -> State {
    let max_stack = core::cmp::max(old.stack.len(), incoming.stack.len());
    let mut stack = Vec::with_capacity(max_stack);
    for i in 0..max_stack {
        let a = old.stack.get(i).copied().unwrap_or(Slot::computed(Interval::BOTTOM));
        let b = incoming.stack.get(i).copied().unwrap_or(Slot::computed(Interval::BOTTOM));
        stack.push(Slot {
            iv: a.iv.join(b.iv),
            origin: if a.origin == b.origin { a.origin } else { Origin::Computed },
        });
    }
    let cap = core::cmp::max(old.locals.len(), incoming.locals.len());
    let mut locals = Vec::with_capacity(cap);
    for i in 0..cap {
        let a = old.locals.get(i).copied().unwrap_or(Slot::top());
        let b = incoming.locals.get(i).copied().unwrap_or(Slot::top());
        locals.push(Slot {
            iv: a.iv.join(b.iv),
            origin: if a.origin == b.origin { a.origin } else { Origin::Computed },
        });
    }
    State { stack, locals }
}

/// The exit (return) state of a CFG: the fold of every `Ret` block's ops,
/// hull-joined over all return blocks. The value the word returns is on the
/// (abstract) stack of this state.
pub fn exit_state(cf: &CfResult, word: &ir::Word, sr: &SubtypeRange<'_>) -> State {
    let mut acc: Option<State> = None;
    for (i, block) in word.blocks.iter().enumerate() {
        let rets = matches!(block.ops.iter().last().map(|op| &op.kind), Some(OpKind::Ret));
        if !rets {
            continue;
        }
        let mut flow = cf
            .states
            .get(i)
            .cloned()
            .unwrap_or_else(|| State::fresh(1));
        for op in block.ops.iter() {
            flow.step(&op.kind, sr);
        }
        acc = Some(match acc {
            None => flow,
            Some(a) => merge_first_visit(&a, &flow),
        });
    }
    acc.unwrap_or_else(|| State::fresh(1))
}

/// One obligation's in-tree discharge from a word CFG analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SiteVerdict {
    pub status: crate::verdict::VerdictStatus,
    pub provably_failing: bool,
    pub reason: Option<String>,
}

/// Discharge the module's `subtype-range` obligations against a word CFG.
///
/// Site-to-value mapping (documented in `ir-op-semantics.md`, §7.2):
/// - `InRange(Var "in.i")` — the i-th callee input, evaluated at the word's
///   ENTRY state. Inputs are `⊤` at entry (the callee cannot know callers),
///   so these resolve `Open` with an unproven reason.
/// - `InRange(Var "out.i")` — the i-th output, evaluated at the EXIT state
///   (the top of the return stack). Constant returns discharge.
/// - `InRange(Cast …)` — evaluated at the occurrence-th *narrowing cast* in
///   CFG order (counted among Cast-shaped formulas only) on its PRE-cast
///   value; the surrounding engine records the pre-cast tops as it walks.
/// - `InRange(Var "$top")` (stores; opaque) — not externally re-derivable:
///   `Open`.
///
/// The emission-time path ([`Linear`]) produces the per-site values directly;
/// this engine is the normative reference the soundness harness compares
/// concrete runs against.
pub fn discharge_word(
    word: &ir::Word,
    obligations: &[crate::model::Obligation],
    sr: &SubtypeRange<'_>,
) -> Vec<SiteVerdict> {
    let cf = run_cfg(word, sr);
    let exit = exit_state(&cf, word, sr);
    let mut cast_occurrence = 0usize;
    let mut seen_casts = Vec::new();
    // Walk the CFG in block order collecting the pre-cast tops of narrowing
    // casts (deterministic IR order).
    for block in word.blocks.iter() {
        let mut flow = State::fresh(1);
        for op in block.ops.iter() {
            // Capture the PRE-cast value before the Cast transfer narrows.
            if let OpKind::Cast { to, .. } = &op.kind {
                if sr(*to).is_some() {
                    seen_casts.push(flow.top_interval());
                }
            }
            flow.step(&op.kind, sr);
        }
    }

    obligations
        .iter()
        .map(|o| {
            use crate::model::{Formula, Oel};
            let (lo, hi, value) = match &o.formula {
                Formula::InRange { value, lo, hi } => (*lo, *hi, value),
                Formula::OffsetLE { .. } => {
                    return SiteVerdict {
                        status: crate::verdict::VerdictStatus::Open,
                        provably_failing: false,
                        reason: Some("mmio-bounds obligations are descriptor-discharged".to_string()),
                    };
                }
                // Contract obligations (slice P6) are resolved at their
                // emission-time sites (the epilogue's abstract verdict for
                // `contract-post`; caller provenance for `contract-pre`) —
                // the reference engine leaves them open.
                Formula::PredicateHolds { .. } => {
                    return SiteVerdict {
                        status: crate::verdict::VerdictStatus::Open,
                        provably_failing: false,
                        reason: Some("contract obligations are resolved at the emission site".to_string()),
                    };
                }
            };
            let iv = match value {
                Oel::Var { name } => match name.as_str() {
                    "in.i" => Interval::TOP, // callee inputs unproven
                    "out.i" => exit
                        .top_n_intervals(word.sig.out_len as usize)
                        .into_iter()
                        .next()
                        .unwrap_or(Interval::TOP),
                    _ => Interval::TOP, // $top: opaque
                },
                Oel::Cast { .. } => {
                    let v = seen_casts.get(cast_occurrence).copied().unwrap_or(Interval::TOP);
                    cast_occurrence += 1;
                    v
                }
            };
            let v = InTreeVerdict::of_range(iv, lo, hi);
            SiteVerdict {
                status: v.status,
                provably_failing: v.provably_failing,
                reason: v.reason,
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// The C4 store-interval path (slice P5): a store's pre-value interval
    /// drives the discharge — a bounded in-range store discharges, a ⊤ store
    /// (e.g. from a `Load`) stays open, and a provably-out-of-range constant
    /// is `provably_failing`.
    #[test]
    fn store_interval_verdicts() {
        let ok = InTreeVerdict::of_range(Interval::Range { lo: 42, hi: 42 }, 0, 100);
        assert_eq!(ok.status, crate::verdict::VerdictStatus::Discharged);
        assert!(!ok.provably_failing);

        let wild = InTreeVerdict::of_range(Interval::TOP, 0, 100);
        assert_eq!(wild.status, crate::verdict::VerdictStatus::Open);
        assert!(!wild.provably_failing, "⊤ store is open, never provably failing");

        let bad = InTreeVerdict::of_range(Interval::Range { lo: 150, hi: 150 }, 0, 100);
        assert_eq!(bad.status, crate::verdict::VerdictStatus::Open);
        assert!(bad.provably_failing, "out-of-range store → provably_failing");
    }

    /// The linear flow through a callee body: a constant return narrows the
    /// output interval (the C2 discharge source).
    #[test]
    fn linear_flow_tracks_constants_through_arithmetic() {
        let mut st = State::callee_entry(0, 4);
        st.step(&OpKind::ConstI64(100), sr_static());
        st.step(&OpKind::ConstI64(50), sr_static());
        st.step(&OpKind::SubI64, sr_static());
        assert_eq!(st.top_interval(), Interval::Range { lo: 50, hi: 50 });
        st.step(&OpKind::ConstI64(3), sr_static());
        st.step(&OpKind::AddI64, sr_static());
        assert_eq!(st.top_interval(), Interval::Range { lo: 53, hi: 53 });
        // A Load cuts the flow to ⊤ (Q4 — memory is not modeled).
        st.step(&OpKind::Load { ty: ir::TypeId(0) }, sr_static());
        assert_eq!(st.top_interval(), Interval::TOP);
    }

    fn sr_static() -> &'static SubtypeRange<'static> {
        &|tid| if tid.0 == 2 { Some((0, 100)) } else { None }
    }
}
