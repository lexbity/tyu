//! The Rust source-fragment interpreter + cross-surface transcription
//! (PLAN-VERIFY-3 P9.2 — test-only support surface).
//!
//! Mirrors the port's pure-fragment embedding (`Tyu/Src.lean`): the fragment
//! op set [`SrcOp`] (values, typed-stack ops, arithmetic/comparison/logic,
//! memory over the oracles, local cells, CFG control), its *direct* concrete
//! step semantics ([`run_block`], [`run_word`]), and the op-local
//! transcription [`transcribe`] into the IR `OpKind` surface. The
//! cross-surface vectors (`crates/verifier/tests/cross_surface.rs`) run the
//! same programs under the source semantics and under the transcribed-IR
//! semantics and require observable-trace agreement — the Rust analogue of
//! the T-S theorem (`Tyu.Src.transcription_run`) on the executed corpus.
//!
//! The op-locality boundary matches `Tyu/Src.lean` exactly: casts, trap
//! sequences, calls, and address materialization are NOT fragment ops (their
//! lowering is not op-local; §Q2's shrink).
//!
//! This module is a documented test-only surface (like `testutil`): it is
//! compiled into the crate so integration tests can reach it, but nothing in
//! the production pipeline consumes it.

use alloc::vec;
use alloc::vec::Vec;

use ir::{BlockId, CmpKind, OpKind, TypeId};

/// The fragment op set (mirror of `Tyu/Src.Op`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SrcOp {
    ConstInt(i64),
    ConstBool(bool),
    Dup,
    Drop,
    Swap,
    Add,
    Sub,
    Mul,
    CmpLt,
    CmpLe,
    CmpGt,
    CmpGe,
    CmpEq,
    CmpNe,
    AndB,
    OrB,
    NotB,
    Load,
    Store,
    VolLoad,
    VolStore,
    LocalGet(u16),
    LocalSet(u16),
    Br(usize),
    BrIf(usize, usize),
    Ret,
}

impl SrcOp {
    /// The canonical `--emit=ir` mnemonic (the `Tyu.IR.OpForm.mnemonic`
    /// subset the fragment covers).
    pub fn mnemonic(&self) -> &'static str {
        match self {
            SrcOp::ConstInt(_) => "const_i64",
            SrcOp::ConstBool(_) => "const_bool",
            SrcOp::Dup => "dup",
            SrcOp::Drop => "drop",
            SrcOp::Swap => "swap",
            SrcOp::Add => "add_i64",
            SrcOp::Sub => "sub_i64",
            SrcOp::Mul => "mul_i64",
            SrcOp::CmpLt => "cmp_lt",
            SrcOp::CmpLe => "cmp_le",
            SrcOp::CmpGt => "cmp_gt",
            SrcOp::CmpGe => "cmp_ge",
            SrcOp::CmpEq => "cmp_eq",
            SrcOp::CmpNe => "cmp_ne",
            SrcOp::AndB => "and_bool",
            SrcOp::OrB => "or_bool",
            SrcOp::NotB => "not_bool",
            SrcOp::Load => "load",
            SrcOp::Store => "store",
            SrcOp::VolLoad => "vol_load",
            SrcOp::VolStore => "vol_store",
            SrcOp::LocalGet(_) => "local_get",
            SrcOp::LocalSet(_) => "local_set",
            SrcOp::Br(_) => "br",
            SrcOp::BrIf(_, _) => "br_if",
            SrcOp::Ret => "ret",
        }
    }
}

/// The op-local transcription: one IR op per fragment construct (mirror of
/// `Tyu.Src.transcribeOp`).
pub fn transcribe(op: &SrcOp) -> OpKind {
    let t = TypeId(0);
    match op {
        SrcOp::ConstInt(v) => OpKind::ConstI64(*v),
        SrcOp::ConstBool(b) => OpKind::ConstBool(*b),
        SrcOp::Dup => OpKind::Dup { ty: t },
        SrcOp::Drop => OpKind::Drop { ty: t },
        SrcOp::Swap => OpKind::Swap { a: t, b: t },
        SrcOp::Add => OpKind::AddI64,
        SrcOp::Sub => OpKind::SubI64,
        SrcOp::Mul => OpKind::MulI64,
        SrcOp::CmpLt => OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Lt,
        },
        SrcOp::CmpLe => OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Le,
        },
        SrcOp::CmpGt => OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Gt,
        },
        SrcOp::CmpGe => OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Ge,
        },
        SrcOp::CmpEq => OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Eq,
        },
        SrcOp::CmpNe => OpKind::Cmp {
            out: TypeId(1),
            kind: CmpKind::Ne,
        },
        SrcOp::AndB => OpKind::AndBool,
        SrcOp::OrB => OpKind::OrBool,
        SrcOp::NotB => OpKind::NotBool,
        SrcOp::Load => OpKind::Load { ty: t },
        SrcOp::Store => OpKind::Store { ty: t },
        SrcOp::VolLoad => OpKind::MmioVolLoad {
            ty: t,
            place: ir::Atom::new(b"dev").unwrap(),
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        },
        SrcOp::VolStore => OpKind::MmioVolStore {
            ty: t,
            place: ir::Atom::new(b"dev").unwrap(),
            write_kind: ir::WriteKind::Plain,
            read_kind: ir::ReadKind::Plain,
            atomic_max: 64,
            barrier: ir::BarrierKind::None,
        },
        SrcOp::LocalGet(slot) => OpKind::LocalGet {
            slot: *slot,
            ty: t,
        },
        SrcOp::LocalSet(slot) => OpKind::LocalSet {
            slot: *slot,
            ty: t,
        },
        SrcOp::Br(target) => OpKind::Br {
            target: BlockId(*target as u16),
        },
        SrcOp::BrIf(then_tgt, else_tgt) => OpKind::BrIf {
            then_tgt: BlockId(*then_tgt as u16),
            else_tgt: BlockId(*else_tgt as u16),
        },
        SrcOp::Ret => OpKind::Ret,
    }
}

// ---------------------------------------------------------------------------
// The shared concrete memory (mirror of `Tyu.Step.ConcreteMem`)
// ---------------------------------------------------------------------------

/// The concrete memory: recorded point stores + the load/MMIO oracles
/// (§Q13 — nondeterminism is a parameter, never a fixed value).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Mem {
    pub cells: Vec<(i64, i64)>,
    pub load_oracle: i64,
    pub mmio_oracle: i64,
}

impl Mem {
    pub fn new(load_oracle: i64, mmio_oracle: i64) -> Mem {
        Mem {
            cells: Vec::new(),
            load_oracle,
            mmio_oracle,
        }
    }

    /// Record a point store (most recent wins — the `find?.first` reading of
    /// `Tyu.Step.ConcreteMem.load`).
    pub fn record(&mut self, addr: i64, val: i64) {
        self.cells.retain(|(a, _)| *a != addr);
        self.cells.insert(0, (addr, val));
    }

    pub fn load(&self, addr: i64) -> i64 {
        self.cells
            .iter()
            .find(|(a, _)| *a == addr)
            .map(|(_, v)| *v)
            .unwrap_or(self.load_oracle)
    }

    pub fn mmio_read(&self) -> i64 {
        self.mmio_oracle
    }
}

// ---------------------------------------------------------------------------
// The concrete state and the run engine (mirror of `Tyu.Src`)
// ---------------------------------------------------------------------------

/// The concrete run state: the value stack + the local cells.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct State {
    pub stack: Vec<i64>,
    pub locals: Vec<i64>,
}

impl State {
    pub fn fresh(locals_cap: usize) -> State {
        State {
            stack: Vec::new(),
            locals: vec![0; locals_cap],
        }
    }

    fn push(&mut self, v: i64) {
        self.stack.push(v);
    }

    /// Pop with saturation at empty (the `State.pop1` default) — the
    /// underflow case never arises in a well-typed run.
    fn pop(&mut self) -> i64 {
        self.stack.pop().unwrap_or(0)
    }
}

/// 64-bit two's-complement wrapping arithmetic (mirror of `Tyu.Step.wrapI64`;
/// Rust's wrapping arithmetic is the same as `x.emod 2^64` signed).
fn add(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}
fn sub(a: i64, b: i64) -> i64 {
    a.wrapping_sub(b)
}
fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}

fn cmp(op: &SrcOp, a: i64, b: i64) -> bool {
    match op {
        SrcOp::CmpLt => a < b,
        SrcOp::CmpLe => a <= b,
        SrcOp::CmpGt => a > b,
        SrcOp::CmpGe => a >= b,
        SrcOp::CmpEq => a == b,
        SrcOp::CmpNe => a != b,
        _ => false,
    }
}

/// The block-routing end of a block step (mirror of `Tyu.Step.Block.End`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum End {
    Ret(State),
    Go(usize, State),
    BrIf(usize, usize, i64, State),
    Trap,
}

/// The source fragment step of one data op (control ops are intercepted by
/// [`run_block`], exactly as the IR block run does). The memory is updated
/// and returned with the outcome state.
fn step_data(mem: &mut Mem, op: &SrcOp, st: &mut State) -> bool {
    match op {
        SrcOp::ConstInt(v) => st.push(*v),
        SrcOp::ConstBool(b) => st.push(if *b { 1 } else { 0 }),
        SrcOp::Dup => {
            let v = *st.stack.last().unwrap_or(&0);
            st.push(v);
        }
        SrcOp::Drop => {
            let _ = st.pop();
        }
        SrcOp::Swap => {
            let b = st.pop();
            let a = st.pop();
            st.push(b);
            st.push(a);
        }
        SrcOp::Add => {
            let b = st.pop();
            let a = st.pop();
            st.push(add(a, b));
        }
        SrcOp::Sub => {
            let b = st.pop();
            let a = st.pop();
            st.push(sub(a, b));
        }
        SrcOp::Mul => {
            let b = st.pop();
            let a = st.pop();
            st.push(mul(a, b));
        }
        SrcOp::CmpLt | SrcOp::CmpLe | SrcOp::CmpGt | SrcOp::CmpGe | SrcOp::CmpEq
        | SrcOp::CmpNe => {
            let b = st.pop();
            let a = st.pop();
            st.push(if cmp(op, a, b) { 1 } else { 0 });
        }
        SrcOp::AndB => {
            let b = st.pop();
            let a = st.pop();
            st.push(if a != 0 && b != 0 { 1 } else { 0 });
        }
        SrcOp::OrB => {
            let b = st.pop();
            let a = st.pop();
            st.push(if a != 0 || b != 0 { 1 } else { 0 });
        }
        SrcOp::NotB => {
            let a = st.pop();
            st.push(if a == 0 { 1 } else { 0 });
        }
        SrcOp::Load => {
            let addr = st.pop();
            let v = mem.load(addr);
            st.push(v);
        }
        SrcOp::Store => {
            // `[addr, value]` — the value is on top.
            let v = st.pop();
            let addr = st.pop();
            mem.record(addr, v);
        }
        SrcOp::VolLoad => {
            let _ = st.pop();
            let v = mem.mmio_read();
            st.push(v);
        }
        SrcOp::VolStore => {
            let _ = st.pop();
            let _ = st.pop();
        }
        SrcOp::LocalGet(slot) => {
            let idx = *slot as usize;
            let v = st.locals.get(idx).copied().unwrap_or(0);
            st.push(v);
        }
        SrcOp::LocalSet(slot) => {
            let v = st.pop();
            let idx = *slot as usize;
            if st.locals.len() > idx {
                st.locals[idx] = v;
            }
        }
        // Control ops are intercepted by `run_block` — never stepped here.
        SrcOp::Br(_) | SrcOp::BrIf(_, _) | SrcOp::Ret => (),
    }
    true
}

/// Run one block's ops (source semantics). `ops` is the fragment op list;
/// the routed end is returned with the final memory.
pub fn run_block(mem: &Mem, ops: &[SrcOp], st: &State) -> (Mem, End) {
    let mut m = mem.clone();
    let mut s = st.clone();
    for op in ops {
        match op {
            SrcOp::Ret => return (m, End::Ret(s)),
            SrcOp::Br(target) => return (m, End::Go(*target, s)),
            SrcOp::BrIf(then_tgt, else_tgt) => {
                let cond = s.pop();
                return (m, End::BrIf(*then_tgt, *else_tgt, cond, s));
            }
            _ => {
                step_data(&mut m, op, &mut s);
            }
        }
    }
    (m, End::Ret(s))
}

/// Run a word's CFG from block `entry` for `fuel` block-steps (source
/// semantics). `none` = the run didn't terminate within the budget (or
/// trapped).
pub fn run_word(blocks: &[Vec<SrcOp>], entry: usize, fuel: usize, mem: &Mem, st: &State) -> Option<State> {
    fn go(
        blocks: &[Vec<SrcOp>],
        entry: usize,
        fuel: usize,
        mem: &Mem,
        st: &State,
    ) -> Option<State> {
        if fuel == 0 {
            return None;
        }
        let ops = blocks.get(entry).map(|b| b.as_slice()).unwrap_or(&[]);
        let old = mem.clone();
        let (m1, end) = run_block(mem, ops, st);
        let _ = old;
        match end {
            End::Ret(s1) => Some(s1),
            End::Go(t, s1) => go(blocks, t, fuel - 1, &m1, &s1),
            End::BrIf(t, e, cond, s1) => {
                if cond != 0 {
                    go(blocks, t, fuel - 1, &m1, &s1)
                } else {
                    go(blocks, e, fuel - 1, &m1, &s1)
                }
            }
            End::Trap => None,
        }
    }
    go(blocks, entry, fuel, mem, st)
}

/// The transcribed-IR side: run the same fragment program through the
/// concrete `ir::OpKind` semantics (the Rust analogue of the IR interpreter
/// the T-S statement compares against). Memory and stack semantics mirror
/// the source side, so the transcription theorem's claim — step-for-step
/// agreement on the fragment — is what the cross-surface vectors assert.
pub fn ir_run_word(
    blocks: &[Vec<SrcOp>],
    entry: usize,
    fuel: usize,
    mem: &Mem,
    st: &State,
) -> Option<State> {
    fn go(
        blocks: &[Vec<SrcOp>],
        entry: usize,
        fuel: usize,
        mem: &Mem,
        st: &State,
    ) -> Option<State> {
        if fuel == 0 {
            return None;
        }
        let ops: Vec<OpKind> = blocks
            .get(entry)
            .map(|b| b.iter().map(transcribe).collect())
            .unwrap_or_default();
        let mut m = mem.clone();
        let mut s = st.clone();
        for kind in &ops {
            match kind {
                OpKind::Ret | OpKind::Br { .. } | OpKind::BrIf { .. } => (),
                _ => ir_step(&mut m, kind, &mut s),
            }
        }
        // routing by the last op
        match ops.last() {
            Some(OpKind::Ret) => Some(s),
            Some(OpKind::Br { target }) => go(blocks, target.0 as usize, fuel - 1, &m, &s),
            Some(OpKind::BrIf { then_tgt, else_tgt }) => {
                let cond = s.pop();
                if cond != 0 {
                    go(blocks, then_tgt.0 as usize, fuel - 1, &m, &s)
                } else {
                    go(blocks, else_tgt.0 as usize, fuel - 1, &m, &s)
                }
            }
            _ => Some(s),
        }
    }
    go(blocks, entry, fuel, mem, st)
}

/// The concrete `OpKind` step (the fragment subset only — the source and IR
/// surfaces must agree; a non-fragment op is a test bug).
fn ir_step(mem: &mut Mem, kind: &OpKind, s: &mut State) {
    match kind {
        OpKind::ConstI64(v) => s.push(*v),
        OpKind::ConstBool(b) => s.push(if *b { 1 } else { 0 }),
        OpKind::Dup { .. } => {
            let v = *s.stack.last().unwrap_or(&0);
            s.push(v);
        }
        OpKind::Drop { .. } => {
            let _ = s.pop();
        }
        OpKind::Swap { .. } => {
            let b = s.pop();
            let a = s.pop();
            s.push(b);
            s.push(a);
        }
        OpKind::AddI64 => {
            let b = s.pop();
            let a = s.pop();
            s.push(add(a, b));
        }
        OpKind::SubI64 => {
            let b = s.pop();
            let a = s.pop();
            s.push(sub(a, b));
        }
        OpKind::MulI64 => {
            let b = s.pop();
            let a = s.pop();
            s.push(mul(a, b));
        }
        OpKind::Cmp { kind, .. } => {
            let b = s.pop();
            let a = s.pop();
            let t = match kind {
                CmpKind::Lt => a < b,
                CmpKind::Le => a <= b,
                CmpKind::Gt => a > b,
                CmpKind::Ge => a >= b,
                CmpKind::Eq => a == b,
                CmpKind::Ne => a != b,
            };
            s.push(if t { 1 } else { 0 });
        }
        OpKind::AndBool => {
            let b = s.pop();
            let a = s.pop();
            s.push(if a != 0 && b != 0 { 1 } else { 0 });
        }
        OpKind::OrBool => {
            let b = s.pop();
            let a = s.pop();
            s.push(if a != 0 || b != 0 { 1 } else { 0 });
        }
        OpKind::NotBool => {
            let a = s.pop();
            s.push(if a == 0 { 1 } else { 0 });
        }
        OpKind::Load { .. } => {
            let addr = s.pop();
            s.push(mem.load(addr));
        }
        OpKind::Store { .. } => {
            let v = s.pop();
            let addr = s.pop();
            mem.record(addr, v);
        }
        OpKind::MmioVolLoad { .. } => {
            let _ = s.pop();
            s.push(mem.mmio_read());
        }
        OpKind::MmioVolStore { .. } => {
            let _ = s.pop();
            let _ = s.pop();
        }
        OpKind::LocalGet { slot, .. } => {
            let idx = *slot as usize;
            s.push(s.locals.get(idx).copied().unwrap_or(0));
        }
        OpKind::LocalSet { slot, .. } => {
            let v = s.pop();
            let idx = *slot as usize;
            if s.locals.len() > idx {
                s.locals[idx] = v;
            }
        }
        _ => (), // control ops are routed by the caller
    }
}

/// Observable-trace agreement: both surfaces must produce the same
/// terminating exit stack (and same recorded memory cells, sorted) for the
/// same fragment program, entry state and budget.
pub fn trace_agrees(
    blocks: &[Vec<SrcOp>],
    entry: usize,
    fuel: usize,
    mem: &Mem,
    st: &State,
) -> bool {
    let src = run_word(blocks, entry, fuel, mem, st);
    let ir = ir_run_word(blocks, entry, fuel, mem, st);
    match (src, ir) {
        (Some(a), Some(b)) => a.stack == b.stack && a.locals == b.locals,
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcription_table_is_fixed() {
        // The op-locality transcription, pinned: one IR op per source op
        // with the identical payload (Tyu.Src.transcribeOp's Rust mirror).
        assert_eq!(transcribe(&SrcOp::ConstInt(42)), OpKind::ConstI64(42));
        assert_eq!(transcribe(&SrcOp::Add), OpKind::AddI64);
        assert_eq!(
            transcribe(&SrcOp::LocalGet(3)),
            OpKind::LocalGet {
                slot: 3,
                ty: TypeId(0)
            }
        );
        assert_eq!(
            transcribe(&SrcOp::BrIf(1, 2)),
            OpKind::BrIf {
                then_tgt: BlockId(1),
                else_tgt: BlockId(2)
            }
        );
        assert_eq!(transcribe(&SrcOp::Ret), OpKind::Ret);
    }

    #[test]
    fn const_word_agrees() {
        // `const 42; ret` — the Sum questionnaire shape of the worked
        // example (`tests/source-fixture/SumFix.lean`).
        let blocks = vec![vec![SrcOp::ConstInt(42), SrcOp::Ret]];
        let mem = Mem::new(0, 0);
        let st = State::fresh(4);
        assert!(trace_agrees(&blocks, 0, 8, &mem, &st));
        let r = run_word(&blocks, 0, 8, &mem, &st).unwrap();
        assert_eq!(r.stack, vec![42]);
    }
}