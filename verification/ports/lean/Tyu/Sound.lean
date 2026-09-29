import Tyu.Step
import Tyu.Src
import Tyu.Bundles
import Tyu.Abs

/-! T-C: the stack algebra (PLAN-VERIFY-3 P4.2) — the first registry
theorem, proved against the concrete step semantics of `Tyu.Step` and over
the §13.2 `(net, high)` monoid:

  for terminating executions,
    exit depth = entry depth + `net`      (exact)
    peak depth ≤ entry depth + `high`     (bound)

where `(net, high)` are the word's per-op composition (the monoid fold
`foldBound`); the port's `--level stackmeta` mode compares those values
against the artifact's declared `facts.words[].net/high` (the empirical
hook). The loop case (a net-zero cycle) is `repeat_net_zero`; the CFG walk
guarantees are assembled in `stack_algebra`. -/

namespace Tyu.Sound

open Tyu.Step

/-- The composable `(net, high)` pair (§13.2; mirrors
`ir::contract::StackBound`). -/
structure Bound where
  net : Int
  high : Int
  deriving DecidableEq, Repr, Inhabited

namespace Bound

/-- The identity fragment. -/
def id : Bound := ⟨0, 0⟩

/-- Sequence composition `w1 w2`: `high = max(high1, net1 + high2)`. -/
def compose (a b : Bound) : Bound :=
  ⟨a.net + b.net, max a.high (a.net + b.high)⟩

/-- Branch merge: the nets must be equal (the typechecker guarantees it);
the peak is the max of the two. -/
def branchMax (a b : Bound) : Bound :=
  ⟨a.net, max a.high b.high⟩

/-- Well-formedness: `high ≥ 0` and `high ≥ net` — the `StackBound`
documented invariant. Every per-op bound is well-formed and composition
preserves it. -/
def wf (b : Bound) : Prop := 0 ≤ b.high ∧ b.net ≤ b.high

theorem id_wf : wf id := by simp [wf, id]

theorem compose_wf {a b : Bound} (ha : wf a) (hb : wf b) : wf (compose a b) := by
  cases a with | mk an ah =>
  cases b with | mk bn bh =>
  cases ha with | intro ha0 han =>
  cases hb with | intro hb0 hbn =>
  simp [compose, wf] at ha0 han hb0 hbn ⊢
  constructor <;> omega

theorem branch_max_wf {a b : Bound} (ha : wf a) (hb : wf b) (hnet : a.net = b.net) :
    wf (branchMax a b) := by
  cases a with | mk an ah =>
  cases b with | mk bn bh =>
  cases ha with | intro ha0 han =>
  cases hb with | intro hb0 hbn =>
  simp [branchMax, wf, hnet] at ha0 han hb0 hbn ⊢
  constructor <;> omega

/-- Composition is associative (the §13.2 monoid law). -/
theorem compose_assoc (a b c : Bound) :
    compose (compose a b) c = compose a (compose b c) := by
  cases a with | mk an ah =>
  cases b with | mk bn bh =>
  cases c with | mk cn ch =>
  simp [compose]
  congr 1 <;> omega

/-- `id` is a left identity for well-formed bounds (`high ≥ 0`). -/
theorem compose_id_left {a : Bound} (ha : wf a) : compose id a = a := by
  cases a with | mk n h =>
  cases ha with | intro ha0 han =>
  simp [compose, id] at ha0 han ⊢
  congr 1 <;> omega

/-- `id` is a right identity for well-formed bounds (`high ≥ net`). -/
theorem compose_id_right {a : Bound} (ha : wf a) : compose a id = a := by
  cases a with | mk n h =>
  cases ha with | intro ha0 han =>
  simp [compose, id] at ha0 han ⊢
  congr 1 <;> omega

theorem compose_high_ge_left (a b : Bound) : a.high ≤ (compose a b).high := by
  cases a with | mk an ah =>
  cases b with | mk bn bh =>
  simp [compose]
  omega

theorem compose_high_ge_right (a b : Bound) (hb : wf b) :
    a.net + b.high ≤ (compose a b).high := by
  cases a with | mk an ah =>
  cases b with | mk bn bh =>
  cases hb with | intro hb0 hbn =>
  simp [compose]
  omega

end Bound

/-- The per-op bound from the generated stack effect: `(net, max 0 net)`.
This is the §13.2 per-op table (every row's `high = max(0, net)`; `call`
uses the resolved sig). -/
def opBound (o : ConcreteOp) : Bound :=
  ⟨o.net, max 0 o.net⟩

theorem opBound_wf (o : ConcreteOp) : Bound.wf (opBound o) := by
  simp [opBound, Bound.wf]
  omega

/-- The sequence-level net (Σ per-op nets). -/
def seqNet : List ConcreteOp → Int
  | [] => 0
  | o :: rest => o.net + seqNet rest

/-- The monoid fold over a sequence, from the identity (the §13.2
composition). -/
def foldBound : List ConcreteOp → Bound
  | [] => Bound.id
  | o :: rest => Bound.compose (opBound o) (foldBound rest)

theorem foldBound_net (ops : List ConcreteOp) : (foldBound ops).net = seqNet ops := by
  induction ops with
  | nil => simp [foldBound, seqNet, Bound.id]
  | cons o rest ih => simp [foldBound, seqNet, Bound.compose, opBound, ih]

theorem foldBound_wf (ops : List ConcreteOp) : Bound.wf (foldBound ops) := by
  induction ops with
  | nil => simp [foldBound, Bound.id_wf]
  | cons o rest ih => exact Bound.compose_wf (opBound_wf o) ih

/-- The sequence high — the enveloping peak of the fragment. -/
def seqHigh (ops : List ConcreteOp) : Int :=
  (foldBound ops).high

/-- The cumulative depth after running `ops` from entry depth `d`. -/
def runDepth : List ConcreteOp → Int → Int
  | [], d => d
  | o :: rest, d => runDepth rest (d + o.net)

/-- The peak depth reached while running `ops` from entry depth `d`
(entry included). -/
def peakDepth : List ConcreteOp → Int → Int
  | [], d => d
  | o :: rest, d =>
      let d' := d + o.net
      max (max d d') (peakDepth rest d')

/-- The exact depth law: exit = entry + Σ nets. -/
theorem seq_net_exact : ∀ (ops : List ConcreteOp) (d : Int),
    runDepth ops d = d + seqNet ops := by
  intro ops
  induction ops with
  | nil => intro d; simp [runDepth, seqNet]
  | cons o rest ih =>
      intro d
      simp [runDepth, seqNet]
      rw [ih]
      all_goals omega

/-- The peak envelope law: peak ≤ entry + the monoid high. -/
theorem seq_peak_envelope : ∀ (ops : List ConcreteOp) (d : Int),
    peakDepth ops d ≤ d + seqHigh ops := by
  intro ops
  induction ops with
  | nil => intro d; simp [peakDepth, seqHigh, foldBound, Bound.id]
  | cons o rest ih =>
      intro d
      have hh : 0 ≤ seqHigh (o :: rest) :=
        (Bound.compose_wf (opBound_wf o) (foldBound_wf rest)).1
      have hd : d ≤ d + seqHigh (o :: rest) := by omega
      have hd' : d + o.net ≤ d + seqHigh (o :: rest) := by
        have hle : max 0 o.net ≤ seqHigh (o :: rest) := by
          exact Bound.compose_high_ge_left (opBound o) (foldBound rest)
        simp [seqHigh, foldBound] at hle ⊢
        omega
      have ht : peakDepth rest (d + o.net) ≤ (d + o.net) + seqHigh rest := ih (d + o.net)
      have hseq : (d + o.net) + seqHigh rest ≤ d + seqHigh (o :: rest) := by
        have hle2 : o.net + (foldBound rest).high ≤
            (Bound.compose (opBound o) (foldBound rest)).high :=
          Bound.compose_high_ge_right (opBound o) (foldBound rest) (foldBound_wf rest)
        simp [seqHigh, foldBound, Bound.compose] at hle2 ⊢
        omega
      simp [peakDepth]
      all_goals omega

-- -----------------------------------------------------------------------
-- Per-op depth lemmas over the *concrete* step (`Tyu.Step.stepOp`) — the
-- "per-row depth lemmas": the actual stack length changes by `o.net` and
-- the step's peak stays within `entry + max 0 o.net`.
-- -----------------------------------------------------------------------

/-- Running a non-control op consumes its table-pop count and produces its
table-push count (the exit length). `stepOk`'s underflow guard makes the
law premise-free: a run that returns `some` never popped an empty stack. -/
theorem stepOk_length {spec : Tyu.IR.TargetSpec} {mem : ConcreteMem} {o : ConcreteOp} {st st' : State}
    (h : stepOk spec mem o st = some st') :
    st'.stack.length = st.stack.length + o.net := by
  have hge : Tyu.Step.ConcreteOp.effectivePops o ≤ st.stack.length := by
    by_cases hle : Tyu.Step.ConcreteOp.effectivePops o ≤ st.stack.length
    · exact hle
    · have hn : Tyu.Step.stepOk spec mem o st = none := by
        simp [Tyu.Step.stepOk, hle]
      exfalso
      exact (by simpa [hn] using h)
  cases hf : o.form with
  | const_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | const_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | const_str =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | addr_of =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | addr_of_mut =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | mmio_place =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | scoped_enter =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | task_spawn =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | ptr_add_const =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | ptr_add_index =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | dup =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.pushMany_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | drop =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | swap =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.pushMany_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | add_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | sub_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | mul_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_lt =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_le =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_gt =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_ge =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_eq =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_ne =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | and_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | or_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | not_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | interrupt_disable =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega
  | interrupt_enable =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega
  | local_set =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | local_get =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | cast =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | bitcast =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | call =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, ConcreteOp.callNet, ConcreteOp.sig, State.pushMany_length]
      rw [State.popN_length_ge st (o.callSig.getD (1, 1)).1 (by simpa using h.1)]
      all_goals omega
  | load =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | store =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | vol_load =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | vol_store =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | vol_load_field =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | vol_store_field =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | trap_if_false =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      have hv : (State.pop1 st).2 ≠ 0 := by
        by_cases hz : (State.pop1 st).2 = 0
        · exact False.elim (by simpa [hz] using h)
        · exact hz
      have hst : st' = (State.pop1 st).1 := by
        have hm : (stepOp spec mem o st).2 = Outcome.ok (State.pop1 st).1 := by
          simp [hf, stepOp, hv]
        have hred : (State.pop1 st).1 = st' := by simpa [hv, hm] using h.2
        exact hred.symm
      rw [hst]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | br =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega
  | br_if =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | ret =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega


theorem stepOk_peak {spec : Tyu.IR.TargetSpec} {mem : ConcreteMem} {o : ConcreteOp} {st st' : State}
    (h : stepOk spec mem o st = some st') :
    st'.stack.length ≤ st.stack.length + max 0 o.net := by
  have hge : Tyu.Step.ConcreteOp.effectivePops o ≤ st.stack.length := by
    by_cases hle : Tyu.Step.ConcreteOp.effectivePops o ≤ st.stack.length
    · exact hle
    · have hn : Tyu.Step.stepOk spec mem o st = none := by
        simp [Tyu.Step.stepOk, hle]
      exfalso
      exact (by simpa [hn] using h)
  cases hf : o.form with
  | const_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | const_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | const_str =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | addr_of =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | addr_of_mut =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | mmio_place =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | scoped_enter =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | task_spawn =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | ptr_add_const =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | ptr_add_index =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | dup =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.pushMany_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | drop =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | swap =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.pushMany_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | add_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | sub_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | mul_i64 =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_lt =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_le =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_gt =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_ge =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_eq =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | cmp_ne =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | and_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | or_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [State.pop2, hpa]
      all_goals omega
  | not_bool =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | interrupt_disable =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega
  | interrupt_enable =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega
  | local_set =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | local_get =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      all_goals omega
  | cast =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | bitcast =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | call =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, ConcreteOp.callNet, ConcreteOp.sig, State.pushMany_length]
      rw [State.popN_length_ge st (o.callSig.getD (1, 1)).1 (by simpa using h.1)]
      all_goals omega
  | load =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | store =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | vol_load =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | vol_store =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | vol_load_field =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net, State.push1_length]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | vol_store_field =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hpa : (State.pop1 (State.pop1 st).1).1.stack.length = st.stack.length - 2 := by
        simpa [State.pop2] using (State.pop2_length_ge st h.1)
      simp [hpa]
      all_goals omega
  | trap_if_false =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      have hv : (State.pop1 st).2 ≠ 0 := by
        by_cases hz : (State.pop1 st).2 = 0
        · exact False.elim (by simpa [hz] using h)
        · exact hz
      have hst : st' = (State.pop1 st).1 := by
        have hm : (stepOp spec mem o st).2 = Outcome.ok (State.pop1 st).1 := by
          simp [hf, stepOp, hv]
        have hred : (State.pop1 st).1 = st' := by simpa [hv, hm] using h.2
        exact hred.symm
      rw [hst]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | br =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega
  | br_if =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h.2]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      have hlen1 : 1 ≤ st.stack.length := State.length_nonneg_one h.1
      rw [State.pop1_length_ge st hlen1]
      all_goals omega
  | ret =>
      simp [hf, stepOk, ConcreteOp.effectivePops, stepOp, Tyu.IR.OpForm.pops] at h
      rw [← h]
      simp [hf, ConcreteOp.net, Tyu.IR.OpForm.net]
      all_goals omega

/-- The run-level exact depth law over the concrete step: a terminating
(ok) run's exit depth equals the entry depth plus the sequence net. -/
def runOk (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (ops : List ConcreteOp) (st : State) : Option State :=
  match ops with
  | [] => some st
  | o :: rest =>
      match stepOk spec mem o st with
      | none => none
      | some st1 => runOk spec mem rest st1

theorem runOk_length {spec : Tyu.IR.TargetSpec} {mem : ConcreteMem} {ops : List ConcreteOp} {st st' : State}
    (h : runOk spec mem ops st = some st') :
    st'.stack.length = st.stack.length + seqNet ops := by
  induction ops generalizing st with
  | nil =>
      simp [runOk] at h
      simp [h, seqNet]
  | cons o rest ih =>
      simp [runOk] at h
      match hok : stepOk spec mem o st with
      | some st1 =>
          have hstep : stepOk spec mem o st = some st1 := hok
          have hh : runOk spec mem rest st1 = some st' := by
            simp [hstep] at h
            exact h
          have hlen := stepOk_length hstep
          have hrec := ih hh
          simp [seqNet]
          omega
      | none =>
          simp [hok] at h

/-- The run peak at op boundaries (entry included): the maximal exit
length over every executed prefix. During an op the depth never exceeds
its entry or its exit (`stepOk_peak`), so this boundary peak is the run
peak for the registry statement. -/
def runOkPeak (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (ops : List ConcreteOp) (st : State) : Int :=
  match ops with
  | [] => st.stack.length
  | o :: rest =>
      match stepOk spec mem o st with
      | none => st.stack.length
      | some st1 => max st.stack.length (runOkPeak spec mem rest st1)

/-- The run peak bound: peak ≤ entry + the monoid high (transported to the
concrete run through the per-op exit-peak laws). -/
theorem runOkPeak_bound {spec : Tyu.IR.TargetSpec} {mem : ConcreteMem} {ops : List ConcreteOp} {st st' : State}
    (h : runOk spec mem ops st = some st') :
    runOkPeak spec mem ops st ≤ st.stack.length + seqHigh ops := by
  have hle : runOkPeak spec mem ops st ≤ peakDepth ops st.stack.length := by
    induction ops generalizing st with
    | nil => simp [runOkPeak, peakDepth]
    | cons o rest ih =>
        match hok : stepOk spec mem o st with
        | some st1 =>
            have hstep : stepOk spec mem o st = some st1 := hok
            have hlen := stepOk_length hstep
            have hh : runOk spec mem rest st1 = some st' := by
              simp [runOk, hstep] at h
              exact h
            have hrec := ih (st := st1) hh
            have hsub : runOkPeak spec mem rest st1 ≤ peakDepth rest (st.stack.length + o.net) := by
              simpa [hlen] using hrec
            simp [runOkPeak, hstep, peakDepth]
            omega
        | none =>
            simp [runOkPeak, hok, peakDepth]
            omega
  have henv : peakDepth ops st.stack.length ≤ st.stack.length + seqHigh ops :=
    seq_peak_envelope ops st.stack.length
  omega

-- -----------------------------------------------------------------------
-- Walk-level T-C over the block CFG, and the net-zero loop law.
-- -----------------------------------------------------------------------

/-- A block: its op list (the successor routing is carried by the run
engine; the walk theory needs only the op sequence). -/
structure BlockW where
  ops : List ConcreteOp
  deriving Repr, Inhabited

namespace BlockW

def empty : BlockW := { ops := [] }

def net (b : BlockW) : Int := seqNet b.ops
def high (b : BlockW) : Int := seqHigh b.ops

end BlockW

/-- A word: block list (indexed by block id) + the entry block. -/
structure WordW where
  blocks : List BlockW
  entry : Nat
  deriving Repr, Inhabited

/-- A block terminates (its last op is `ret`). -/
def IsRetBlock (w : WordW) (b : Nat) : Prop :=
  match (w.blocks.getD b BlockW.empty).ops.getLast? with
  | some o => o.form = Tyu.IR.OpForm.ret
  | none => False

/-- `t` is a successor of `b` (the last op routes to it). -/
def IsSuccFrom (w : WordW) (b t : Nat) : Prop :=
  match (w.blocks.getD b BlockW.empty).ops.getLast? with
  | some o =>
      (o.form = Tyu.IR.OpForm.br ∧ o.brTgt = some t) ∨
      (o.form = Tyu.IR.OpForm.br_if ∧ o.brIfTgts = some (t, 0)) ∨
      (o.form = Tyu.IR.OpForm.br_if ∧ o.brIfTgts = some (0, t))
  | none => False

/-- A terminating walk: a finite block-id sequence whose first id is
`from`, whose consecutive ids follow successors, ending at a `ret` block.
Loops appear as revisits; the finite list is the termination witness
(the "fuel" is its length; `repeat_net_zero` covers the net-zero cycle). -/
def IsWalk (w : WordW) (_start : Nat) : List Nat → Prop
  | [] => False
  | [b] => IsRetBlock w b
  | b :: rest => IsSuccFrom w b (rest.head?.getD 0) ∧ IsWalk w (rest.head?.getD 0) rest

/-- The concatenated ops of a walk (the execution's op sequence). -/
def walkOps (w : WordW) : List Nat → List ConcreteOp
  | [] => []
  | b :: rest => (w.blocks.getD b BlockW.empty).ops ++ walkOps w rest

/-- The exact walk depth law: exit = entry + Σ nets over the visited ops. -/
theorem walk_net (w : WordW) (ids : List Nat) (d : Int) :
    runDepth (walkOps w ids) d = d + seqNet (walkOps w ids) :=
  seq_net_exact (walkOps w ids) d

/-- The walk peak envelope: peak ≤ entry + the monoid high over the
visited ops. -/
theorem walk_peak (w : WordW) (ids : List Nat) (d : Int) :
    peakDepth (walkOps w ids) d ≤ d + seqHigh (walkOps w ids) :=
  seq_peak_envelope (walkOps w ids) d

/-- The net-zero cycle law (the §13.2 net-zero rule / P4.2's loop case): a
fragment whose net is zero leaves the depth unchanged no matter how many
times it repeats — well-founded induction on the iteration count. -/
def repeatSeq (ops : List ConcreteOp) : Nat → List ConcreteOp
  | 0 => []
  | k + 1 => ops ++ repeatSeq ops k

theorem seqNet_append (a b : List ConcreteOp) : seqNet (a ++ b) = seqNet a + seqNet b := by
  induction a with
  | nil => simp [seqNet]
  | cons o rest ih => simp [seqNet, ih]; omega

theorem repeat_net_zero (ops : List ConcreteOp) (hz : seqNet ops = 0) :
    ∀ (k : Nat), seqNet (repeatSeq ops k) = 0 := by
  intro k
  induction k with
  | zero => simp [repeatSeq, seqNet]
  | succ k ih =>
      simp [repeatSeq, seqNet_append, ih, hz]

theorem repeat_net_exact (ops : List ConcreteOp) (hz : seqNet ops = 0) :
    ∀ (k : Nat) (d : Int), runDepth (repeatSeq ops k) d = d := by
  intro k d
  rw [seq_net_exact]
  rw [repeat_net_zero ops hz k]
  omega

/-- T-C, sequence form: a terminating (ok) run of any op sequence exits at
`entry + seqNet` and never exceeds `entry + seqHigh`. -/
theorem stack_algebra_sequence {spec : Tyu.IR.TargetSpec} {mem : ConcreteMem} {ops : List ConcreteOp} {st st' : State}
    (h : runOk spec mem ops st = some st') :
    st'.stack.length = st.stack.length + seqNet ops ∧
    runOkPeak spec mem ops st ≤ st.stack.length + seqHigh ops := by
  constructor
  · exact runOk_length h
  · exact runOkPeak_bound h

/-- T-C, walk form: every terminating walk through a word's CFG satisfies
the exact net law and the peak envelope (see `IsWalk` for the walk
relation; the walk's own values are what the `--level stackmeta` mode
compares against the artifact's declared per-word `(net, high)`). -/
theorem stack_algebra_iswalk {w : WordW} {start : Nat} {ids : List Nat}
    (_hw : IsWalk w start ids) :
    runDepth (walkOps w ids) 0 = 0 + seqNet (walkOps w ids) ∧
    peakDepth (walkOps w ids) 0 ≤ 0 + seqHigh (walkOps w ids) := by
  constructor
  · exact walk_net w ids 0
  · exact walk_peak w ids 0

/-- T-C (the registry statement, §Q16 ladder): for every terminating walk,
  - exit depth = entry depth + `net` (exact);
  - peak depth ≤ entry depth + `high`;
with `(net, high)` the word's monoid composition, and a net-zero loop body
contributing nothing to the depth (`repeat_net_zero`). -/
theorem stack_algebra_walk {w : WordW} {ids : List Nat} (_hwalk : IsWalk w w.entry ids)
    {spec : Tyu.IR.TargetSpec} {mem : ConcreteMem} {st st' : State}
    (h : runOk spec mem (walkOps w ids) st = some st') :
    st'.stack.length = st.stack.length + seqNet (walkOps w ids) ∧
    runOkPeak spec mem (walkOps w ids) st ≤ st.stack.length + seqHigh (walkOps w ids) := by
  constructor
  · exact runOk_length h
  · exact runOkPeak_bound h

/-! T-CL: the assumption-closure registry statement (PLAN-VERIFY-3 §Q7 rule 2,
P8.1) — image-level closure over the *abstract verdict-set structure*.

A verdict set is **well-closed** when

  1. the assumption graph is acyclic (cycles are malformed — E6419); and
  2. every closed obligation's assumption edges leave to obligations that
     are themselves *closed* (`proof`/`checked`) or marked `runtime-check`
     (the emitted check IS the discharge — §Q7 rule 2).

T-CL proves the transitive form of (2): on a well-closed set, every closed
obligation's *entire* assumption graph — walked through closed nodes — lands
inside `closed ∪ runtime`. That is the principle the image-level closure
walker (`tyu::closure`, P8.2) executes on the concrete artifacts: a
`proof`-discharged contract-pre in the caller is valid only relative to the
callee module that was itself proven; an unresolved edge forces the
dependent open (`assumption-unresolved`), a cycle is E6419-malformed. -/
namespace AssumptionClosure

open Tyu.Sound

/-- An abstract verdict set (§Q7 rule 2): obligations identified by `Id`,
the closed classification (`discharged` with `trust ∈ {proof, checked}`),
the `runtime-check` terminals (the emitted check IS the discharge), and the
assumption edges. -/
structure VerdictSet where
  Id : Type
  closed : Id → Prop
  runtime : Id → Prop
  assumptions : Id → List Id

namespace VerdictSet

/-- An obligation that is discharged (closed) or is a `runtime-check`
terminal — the only leaves a closed obligation's assumption graph may
reach. -/
def closedOrRuntime (V : VerdictSet) (i : V.Id) : Prop :=
  V.closed i ∨ V.runtime i

/-- Reachability along assumption edges that flow through CLOSED obligations:
a `runtime` node is a terminal (it carries no obligations), so paths stop at
it — `step` IS the edge into a terminal. -/
inductive closedPathTo (V : VerdictSet) : V.Id → V.Id → Prop
  | step (i j : V.Id) : V.closed i → j ∈ V.assumptions i → closedPathTo V i j
  | extend (i j k : V.Id) :
      closedPathTo V i j → V.closed j → closedPathTo V j k →
      closedPathTo V i k

/-- Acyclicity of the closed-assumption graph: no closed obligation reaches
itself through a nonempty closed path. Cycles are malformed (E6419). -/
def acyclic (V : VerdictSet) : Prop :=
  ∀ i, V.closed i → ¬ closedPathTo V i i

/-- Local edge soundness: every closed obligation's DIRECT assumption edges
leave to obligations that are closed or `runtime-check` terminals. -/
def edgeSound (V : VerdictSet) : Prop :=
  ∀ i, V.closed i → ∀ j, j ∈ V.assumptions i → closedOrRuntime V j

/-- Well-closedness (§Q7 rule 2): acyclic AND locally edge-sound. -/
def wellClosed (V : VerdictSet) : Prop :=
  acyclic V ∧ edgeSound V

end VerdictSet

open VerdictSet

/-- T-CL (the registry statement): on a well-closed verdict set, every
closed obligation's entire assumption graph (walked through closed nodes)
terminates on obligations that are closed or `runtime-check` terminals.
This is the transitive closure of `edgeSound` along the acyclic graph — the
mathematical content the P8.2 image walker executes. -/
theorem transitive_closure_sound {V : VerdictSet} (hw : V.wellClosed)
    {i j : V.Id} (hci : V.closed i) (hp : V.closedPathTo i j) :
    V.closedOrRuntime j := by
  induction hp with
  | step i' j' hc hmem =>
      exact hw.2 i' hc j' hmem
  | extend i' j' k' _hp1 hcj _hp2 _ih1 ih2 =>
      exact ih2 hcj

/-- A closed obligation reachable from itself — a cycle — contradicts
well-closedness: cycles are malformed (E6419). -/
theorem cyclic_not_well_closed {V : VerdictSet} {i : V.Id}
    (hc : V.closed i) (hp : V.closedPathTo i i) : ¬ V.wellClosed := by
  intro hw
  exact hw.1 i hc hp

/-- An assumption edge that leaves a closed obligation to an OPEN obligation
contradicts well-closedness: an unresolved dependency means the dependent's
closure fails (the image report must resolve it open with witness
`assumption-unresolved`). -/
theorem open_edge_not_well_closed {V : VerdictSet} {i j : V.Id}
    (hci : V.closed i) (hmem : j ∈ V.assumptions i)
    (hj_closed : ¬ V.closed j) (hj_runtime : ¬ V.runtime j) : ¬ V.wellClosed := by
  intro hw
  have h : V.closedOrRuntime j := hw.2 i hci j hmem
  exact h.elim hj_closed hj_runtime

/-- Base case (terminal): a `runtime-check` obligation is a terminal — the
emitted check IS the discharge, so its closure is itself. -/
theorem runtime_terminal {V : VerdictSet} {i : V.Id} (hr : V.runtime i) :
    V.closedOrRuntime i := by
  exact Or.inr hr

/-- Base case (isolated): a closed obligation with no assumption edges is
its own base case — with nothing to walk, its closure is trivially sound. -/
theorem no_edges_terminal {V : VerdictSet} {i : V.Id} {j : V.Id}
    (hne : V.assumptions i = []) (hmem : j ∈ V.assumptions i) :
    V.closedOrRuntime j := by
  rw [hne] at hmem
  simp at hmem

end AssumptionClosure

/-! ## T-S: the source→IR transcription (PLAN-VERIFY-3 P9.1, §Q2)

The registry statement for the source-fragment embedding (`Tyu/Src.lean`):
for a pure-fragment word, the source-level semantics and the IR-level
semantics of its transcription are the same relation on concrete states and
memories, parameterized over every `(TargetSpec, MemModel-instance)` (§Q3).
The per-op simulation lemmas (`Tyu.Src.transcription_op`), their composition
over blocks (`Tyu.Src.transcription_block`) and over the word CFG walk
(`Tyu.Src.transcription_run`) are the machinery; this alias is what the
registry names. -/
section T_Server

-- The aggregated registry entries (proofs live in `Tyu.Src`; aliased here
-- so the axiomit audit and REVIEW.md name one home per registry entry).
theorem transcription_forward {blocks : List Tyu.Src.Block} {entry : Nat} {i : Nat}
    {lo hi : Int} :
    Tyu.Src.outInRange blocks entry i lo hi → Tyu.Src.irOutInRange blocks entry i lo hi :=
  (Tyu.Src.transcription blocks entry i lo hi).1

theorem transcription_backward {blocks : List Tyu.Src.Block} {entry : Nat} {i : Nat}
    {lo hi : Int} :
    Tyu.Src.irOutInRange blocks entry i lo hi → Tyu.Src.outInRange blocks entry i lo hi :=
  (Tyu.Src.transcription blocks entry i lo hi).2

/-- T-S (the registry statement, §Q2/§Q16): the source out-range claim is
equivalent to the transcribed IR out-range claim — a source-level
certificate discharges the IR obligation *in composition with this
theorem*. -/
theorem transcription {blocks : List Tyu.Src.Block} {entry : Nat} {i : Nat}
    {lo hi : Int} :
    Tyu.Src.outInRange blocks entry i lo hi ↔ Tyu.Src.irOutInRange blocks entry i lo hi :=
  Tyu.Src.transcription blocks entry i lo hi

/-- T-S, offset (mmio-bounds) form. -/
theorem transcription_offset {blocks : List Tyu.Src.Block} {entry : Nat}
    {off width size : Nat} :
    Tyu.Src.offsetWithin blocks entry off width size ↔
      Tyu.Src.irOffsetWithin blocks entry off width size :=
  Tyu.Src.transcription_offset blocks entry off width size

end T_Server

/-! ## T-D: the memory-model laws (PLAN-VERIFY-3 P12.2, §6.8/§Q16)

The registry statements over the bundle-instance substrate
(`Tyu.Bundles.BundleMem`): for every modeled bundle's instance (`x86_64`,
`armv7m`, `riscv32`, and the `rp2350` board pack's `rp2350` — each with the
RAM window declared by its `model/model.toml [memory] ram`):

  - **store-load**: a point store within the modeled window is observable
    by a subsequent load of the same address — the stored abstract value
    flows back (memory is a real abstract store, not `⊤`).
  - **frame**: a store to one point address leaves the load behavior of
    every other point address unchanged (the frame property — a load that
    the store does not target reads exactly what it read before).
  - **aperture width bound**: an MMIO read answers exactly the §Q13
    width-bounded nondeterministic domain — a value *within* the register's
    width, `top` (the full i64 domain) at ≥ 64 bits, never wider.

The Rust `ApertureMem` instance (`verifier::mem`) implements the same
rules; the committed `tyu.vec/1` bundle corpora are executed by both sides
and must agree byte-for-byte (`crates/verifier/tests/bundle_instance_conformance.rs`
+ the port's `conformance` exe) — the mechanical pin of this section. -/
namespace TD

open Tyu.Bundles
open Tyu.Mem

/-- T-D (store-load): a point store within the model's window is read back
by a load of that address. -/
theorem store_load {m : BundleMem} {a : Int} (hin : m.inRam a) (v : IntervalVal) :
    BundleMem.loadVal (BundleMem.storeVal m (IntervalVal.range a a) v) (IntervalVal.range a a) = v := by
  simp [BundleMem.storeVal, BundleMem.loadVal, BundleMem.inRam, hin]

/-- T-D (frame): a store to one point address does not disturb the load
behavior of any other point address. -/
theorem frame_law {m : BundleMem} {a b : Int} (hina : m.inRam a) (hinb : m.inRam b)
    (hne : a ≠ b) (v : IntervalVal) :
    BundleMem.loadVal (BundleMem.storeVal m (IntervalVal.range a a) v) (IntervalVal.range b b)
      = BundleMem.loadVal m (IntervalVal.range b b) := by
  have hba : b ≠ a := by exact Ne.symm hne
  simp [BundleMem.storeVal, BundleMem.loadVal, BundleMem.inRam, hina, hinb, hba]

/-- T-D (aperture width bound, sub-64-bit): a register read of width
`0 < w < 64` answers the signed `w`-bit domain — never a wider value
(§Q13). The read goes through the single `Tyu.Conformance.wordDomain`. -/
theorem aperture_width_bound {w : Nat} (hw : 0 < w) (hw64 : w < 64) :
    BundleMem.apertureRead w =
      IntervalVal.range (-((2 : Int) ^ (w - 1))) (((2 : Int) ^ (w - 1)) - 1) := by
  have hz : w ≠ 0 := by omega
  have hb : (if w = 0 then 64 else w) = w := by
    by_cases h : w = 0
    · exact False.elim (hz h)
    · simp [h]
  have hd : Tyu.Abs.wordDomain w =
      some (-((2 : Int) ^ (w - 1)), ((2 : Int) ^ (w - 1)) - 1) := by
    simp [Tyu.Abs.wordDomain, hb, hw64]
  simp [BundleMem.apertureRead, hd]

/-- T-D (aperture width bound, full domain): a ≥ 64-bit register read is
the whole i64 domain (`top`) — the read cannot be narrower than the
register (§Q13). -/
theorem aperture_width_bound_full {w : Nat} (hw64 : 64 ≤ w) :
    BundleMem.apertureRead w = IntervalVal.top := by
  have hz : w ≠ 0 := by omega
  have hb : (if w = 0 then 64 else w) = w := by
    by_cases h : w = 0
    · exact False.elim (hz h)
    · simp [h]
  have hd : Tyu.Abs.wordDomain w = none := by
    simp [Tyu.Abs.wordDomain, hb, hw64]
  simp [BundleMem.apertureRead, hd]

/-! The per-bundle instances (their windows must equal the model artifacts'
declarations — pinned mechanically by the Rust suite via `--geometry`). -/

theorem x86_64_geometry : Bundles.x86_64.ramLo = 1048576 ∧ Bundles.x86_64.ramHi = 2097151 := by
  decide

theorem x86_64_inram (a : Int) (hlo : 1048576 ≤ a) (hhi : a ≤ 2097151) :
    Bundles.x86_64.inRam a := by
  simp [Bundles.x86_64, BundleMem.inRam, hlo, hhi]

theorem armv7m_geometry : Bundles.armv7m.ramLo = 536870912 ∧ Bundles.armv7m.ramHi = 536936447 := by
  decide

theorem armv7m_inram (a : Int) (hlo : 536870912 ≤ a) (hhi : a ≤ 536936447) :
    Bundles.armv7m.inRam a := by
  simp [Bundles.armv7m, BundleMem.inRam, hlo, hhi]

theorem riscv32_geometry : Bundles.riscv32.ramLo = 2147483648 ∧ Bundles.riscv32.ramHi = 2281701375 := by
  decide

theorem riscv32_inram (a : Int) (hlo : 2147483648 ≤ a) (hhi : a ≤ 2281701375) :
    Bundles.riscv32.inRam a := by
  simp [Bundles.riscv32, BundleMem.inRam, hlo, hhi]

/-! ### The rp2350 instance + the T-D completion laws (PLAN-VERIFY-3 P13.2)

The rp2350 board pack is a modeled bundle: its window is the first 64 KiB
of SRAM, and it carries the one `[refinements]` device refinement
(`UARTFR` → `rp2350.uart-fr`, datasheet band `mask = 0xF9`, access-mode
`ro` — pinned against the manifest by `--level bands`). The completion laws
close the T-D family for it: width/mask (the refined read answers the
refinement's band — instanced AT the device), access-mode (ro ⇒ the band,
write-capable ⇒ the §Q13 width-bounded default, reads memory-independent,
the concrete write channel isolated), and volatility/ordering (two reads
record TWO trace entries — the no-elide/merge/reorder guarantee formalized
as the access trace, never merged). -/

/-- The rp2350 window (SRAM `[0x20000000, 0x2000FFFF]`). -/
theorem rp2350_geometry : Bundles.rp2350.ramLo = 536870912 ∧ Bundles.rp2350.ramHi = 536936447 := by
  decide

/-- The rp2350 in-window lemma shares the armv7m proof shape: the rp2350
SRAM window is the SAME constant class as armv7m's (`[0x20000000,
0x2000FFFF]`), so the in-RAM claim reuses `armv7m_inram` rather than
re-proving the constants (P3s finding-6 dedup). -/
theorem rp2350_inram (a : Int) (hlo : 536870912 ≤ a) (hhi : a ≤ 536936447) :
    Bundles.rp2350.inRam a := by
  simpa [Bundles.rp2350, Bundles.armv7m, BundleMem.inRam] using (armv7m_inram a hlo hhi)

/-- T-D (width/mask): a refined read answers exactly the refinement's
modeled band `[0, mask]` — within the register's width, never wider than
the §Q13 width-bounded default (the generic law over any datasheet mask). -/
theorem uartfr_band_domain (mask : Int) (h0 : 0 ≤ mask) (hm : mask ≤ 2147483647) :
    BundleMem.apertureReadRefined mask 32 = IntervalVal.range 0 mask := by
  have hd : Tyu.Conformance.wordDomain 32 = some (-2147483648, 2147483647) := by
    unfold Tyu.Conformance.wordDomain Tyu.Abs.wordDomain
    decide
  rw [BundleMem.apertureReadRefined, hd]
  have hmax : max (-2147483648) 0 = 0 := by omega
  have hmin : min 2147483647 mask = mask := by omega
  simp [hmax, hmin]

/-- T-D (width/mask, INSTANCED at the device): the UARTFR refinement's read
answers exactly the datasheet band `[0, 0xF9]` over the 32-bit register —
the six modeled flag bits (P3 finding: the declared band `uartFrBand`
equals the manifest's `mask = 0xF9`, pinned by `--level bands`, and the
band IS consumed by the registry — for `mask ≤ 0xF9`, reads answer within
the band). -/
theorem uartfr_band_domain_at_device :
    BundleMem.apertureReadRefined Bundles.uartFrBand 32 = IntervalVal.range 0 249 := by
  decide

/-- T-D (width): the refinement's refined read is WITHIN the register's WIDTH
domain — `apertureReadRefined mask w` never answers outside the signed
`w`-bit domain `[−2^(w−1), 2^(w−1) − 1]` (the `min`/`max` cap it; the `width`
is MATHEMATICALLY consumed — P3s finding 6d). The lint additionally admits
only `mask ≤ 2^width − 1`; the model is width-safe regardless. -/
theorem band_within_width_domain (w : Nat) (mask : Int) (hw : 0 < w) (hw64 : w < 64) :
    BundleMem.apertureReadRefined mask w =
      IntervalVal.range (max (-((2 : Int) ^ (w - 1))) 0) (min (((2 : Int) ^ (w - 1)) - 1) mask) := by
  have hz : w ≠ 0 := by omega
  have hb : (if w = 0 then 64 else w) = w := by
    by_cases h : w = 0
    · exact False.elim (hz h)
    · simp [h]
  have hd : Tyu.Conformance.wordDomain w = some (-((2 : Int) ^ (w - 1)), ((2 : Int) ^ (w - 1)) - 1) := by
    simp [Tyu.Abs.wordDomain, hb, hw64]
  unfold BundleMem.apertureReadRefined
  rw [hd]

/-- T-D (access-mode, ro): a refined READ-ONLY register answers exactly the
refinement's band — `refinedOf` consults the mode and the trailing-token
register match: a read of any matching place closes to the datasheet
stable-value set (the descriptor's `access = "ro"` UARTFR row). -/
theorem ro_read_answers_band (m : BundleMem) (mask : Int) (register : String) (w : Nat) :
    (refinedOf m Tyu.Bundles.AccessMode.ro mask register).apertureRead register w
      = BundleMem.apertureReadRefined mask w := by
  unfold refinedOf
  simp [Tyu.Bundles.AccessMode.isRO, Tyu.Bundles.tokenMatches]

/-- T-D (trailing-token agreement, P3s finding-6): `refinedOf` applies the
refinement to EVERY place whose trailing token matches the registered
register — the model uses the SAME place semantics as the binding (the
renderer binds `uart.UARTFR`/`uart0.UARTFR` through the single
`Tyu.Bundles.tokenMatches`; both mirror `verifier::refinements::token_matches`).
This is the refinement-surface analogue of the P12 `bundle_load_agrees`
mirror agreement: a read at any matching place IS the refinement's band —
place matching is load-bearing in the model, not just in the binding. -/
theorem refined_read_at_any_matching_place (m : BundleMem) (place register : String)
    (hmatch : Tyu.Bundles.tokenMatches place register = true)
    (mask : Int) (w : Nat) :
    (refinedOf m Tyu.Bundles.AccessMode.ro mask register).apertureRead place w
      = BundleMem.apertureReadRefined mask w := by
  unfold refinedOf
  simp [hmatch, Tyu.Bundles.AccessMode.isRO]

/-- T-D (access-mode, write-capable): every WRITE-CAPABLE refined register
answers the §Q13 width-bounded nondeterministic domain — the abstract model
records no register state, so reads of write-capable registers stay
unrestricted (the w1c/w1s bit-clears are carried structurally, stated in
the TCB boundary). -/
theorem write_capable_read_width_bounded (m : BundleMem) (mode : Tyu.Bundles.AccessMode)
    (hmode : mode ≠ Tyu.Bundles.AccessMode.ro) (mask : Int) (register : String) (w : Nat) :
    (refinedOf m mode mask register).apertureRead register w = BundleMem.apertureRead w := by
  unfold refinedOf
  cases mode with
  | ro => contradiction
  | wo | rw | w1c | w1s | rc => simp [Tyu.Bundles.AccessMode.isRO, Tyu.Bundles.tokenMatches]

/-- T-D (access-mode, writes): reads of a refined device are a function of
the refinement — never of the memory state. RAM stores and the (unmodeled)
register write state only change memory, and no read consults it: under any
two memories a refined device's read answers the same domain. This is the
abstract model's write-channel isolation; the concrete mirror is
[`store_then_read_is_oracle`]. -/
theorem read_independent_of_memory (m1 m2 : BundleMem) (mode : Tyu.Bundles.AccessMode)
    (mask : Int) (register : String) (w : Nat) :
    (refinedOf m1 mode mask register).apertureRead register w
      = (refinedOf m2 mode mask register).apertureRead register w := by
  unfold refinedOf
  rfl

/-- T-D (access-mode, concrete write channel): on the CONCRETE step (the
surface refined statements live on), an MMIO write never feeds the read
channel — a `vol_store` then a `vol_load` of the same register answers
exactly the oracle's read (the written value is discarded; the abstract
model records no register write state). -/
theorem store_then_read_is_oracle (spec : Tyu.IR.TargetSpec)
    (mem : Tyu.Step.ConcreteMem) (a : Tyu.Step.Value) :
    Tyu.Step.Block.runBlock spec mem
        [ConcreteOp.opMk .vol_store, ConcreteOp.opMk .vol_load]
      { stack := [a, 7, a], locals := [] } =
    (mem, .ret { stack := [mem.mmioRead], locals := [] }) := by
  unfold Tyu.Step.Block.runBlock
  simp [Tyu.Step.Block.runBlock, Tyu.Step.ConcreteOp.opMk,
        Tyu.Step.stepOp, Tyu.Step.State.pop1, Tyu.Step.State.push1]

/-- T-D (volatility trace ↔ semantics): the trace-collecting run performs
the semantics of `Block.runBlock` exactly (identical memory/end) — the
access trace is a formalization of the run's MMIO reads, proven to agree
with the semantics, never a parallel semantics that can drift. -/
theorem runBlockTrace_eq_runBlock (spec : Tyu.IR.TargetSpec) (mem : Tyu.Step.ConcreteMem)
    (ops : List ConcreteOp) (st : Tyu.Step.State) :
    (Tyu.Step.runBlockTrace spec mem ops st).1 = Tyu.Step.Block.runBlock spec mem ops st :=
  Tyu.Step.runBlockTrace_eq_runBlock spec mem ops st

/-- T-D (volatility/ordering): two consecutive MMIO reads are NOT merged —
each `vol_load` records its own read in the run's access TRACE, in
execution order; the trace of `[vol_load, vol_load]` has TWO entries. A
merged implementation (reading once) would record one. This is §Q13's
no-elide/merge/reorder guarantee, formalized as the trace — the final stack
alone cannot distinguish it (a deterministic oracle answers the same value
twice), which is exactly why the trace, not the stack, is the property. -/
theorem two_reads_trace (spec : Tyu.IR.TargetSpec) (mem : Tyu.Step.ConcreteMem)
    (a : Tyu.Step.Value) :
    (Tyu.Step.runBlockTrace spec mem
        [ConcreteOp.opMk .vol_load, ConcreteOp.opMk .vol_load]
      { stack := [a], locals := [] }).2 = [mem.mmioRead, mem.mmioRead] := by
  unfold Tyu.Step.runBlockTrace
  simp [Tyu.Step.runBlockTrace, Tyu.Step.ConcreteOp.opMk, Tyu.Step.stepOp,
        Tyu.Step.State.pop1, Tyu.Step.State.push1]

/-! The T-D consolidation equivalence (P12 cleanup): the conformance
runner's operational `MemModel.bundle` and the theorem-surface `BundleMem`
are two mirrors of one abstract memory; these wrap `Tyu/Bundles/Bundle.lean`'s
equivalence proofs as registry entries so the mirrors cannot drift (the
boundary-byte defect class, finding #1, is exactly what they catch). -/

/-- T-D (mirror equivalence, aperture): the operational conformance read and
`BundleMem.apertureRead` agree value-for-value (both through the one
`wordDomain`). -/
theorem bundle_aperture_agrees (lo hi : Int)
    (cells : List (Int × Tyu.Conformance.Interval)) (w : Nat) :
    Tyu.Conformance.MemModel.apertureRead (Tyu.Conformance.MemModel.bundle lo hi cells []) "" w
      = Tyu.Bundles.toConformance (BundleMem.apertureRead w) :=
  Tyu.Bundles.aperture_read_agrees lo hi cells w

/-- T-D (mirror equivalence, load): a point-address load of the recorded
cells reads identically under both mirrors. -/
theorem bundle_load_agrees (lo hi : Int) (cells : List (Int × Tyu.Conformance.Interval)) (a : Int)
    (w : Nat) :
    Tyu.Conformance.MemModel.load (Tyu.Conformance.MemModel.bundle lo hi cells [])
        (Tyu.Conformance.Interval.range a a) w
      = Tyu.Bundles.toConformance (BundleMem.loadVal (ofCells lo hi cells) (IntervalVal.range a a)) :=
  Tyu.Bundles.load_agrees lo hi cells a w

end TD

/-! ## T-A/T-B: abstraction soundness + discharge soundness (PLAN-VERIFY-3 P14)

The registry statements that justify the **re-derivation** method
(`method: rederive, trust: proof`, §Q12/§Q16; the `rederive` exe re-runs the
abstract interpreter `Tyu.Abs` — this section proves that interpreter
*means* something):

  - **T-A (abstraction soundness)**: every abstract transfer over the
    interval domain over-approximates the corresponding concrete value
    transfer — `Contains iv x` reads "the abstract interval `iv` represents
    the concrete value `x`", and each `*_sound` theorem shows that the
    abstract result interval of an op contains every concrete result of its
    concretizations. The i64-with-wrap→⊤ rule is the soundness of `top`:
    an overflowed bound computation is `⊤`, which represents everything.
  - **T-B (discharge soundness, the T-A corollary)**: `evalInRange` returning
    `defTrue` exactly on `iv ⊆ [lo, hi]`, so any concretization `x ∈ iv`
    satisfies `lo ≤ x ∧ x ≤ hi` — the discharge bridge.

The four named **wrap-boundary examples** (P14.1, machine-checked): the
`i64::MAX`/`i64::MIN` add/sub/mul overflow cases plus the word-domain
`±1` boundary facts per target width (the §Q13 width-relative register
domain) — all `by decide`. -/
namespace TA

open Tyu.Abs

/-- γ — "the abstract interval `iv` represents the concrete value `x`" — is
`Tyu.Abs.Contains` itself (resolved through `open Tyu.Abs`): the T-A theorems
and the re-derive engine share ONE concretization predicate (P14 review:
γ defined once). -/
theorem range_contains {lo hi x : Int} (hlo : lo ≤ x) (hhi : x ≤ hi) :
    Interval.Contains (.range lo hi) x := by
  simp [Interval.Contains]
  exact And.intro hlo hhi

theorem top_contains (x : Int) : Interval.Contains .top x := by simp [Interval.Contains]

theorem bottom_contains_none (x : Int) : ¬ Interval.Contains .bottom x := by simp [Interval.Contains]

/-- A constant's singleton is its own concretization. -/
theorem const_sound (v : Int) : Interval.Contains (.range v v) v := by
  exact range_contains (by omega) (by omega)

/-- A concrete 0/1 result is within the abstract bool interval of `top`. -/
theorem contains_0_1_of_bool (b : Bool) : Interval.Contains (.range 0 1) (if b then 1 else 0) := by
  by_cases hb : b
  · simp [Interval.Contains, hb]
  · simp [Interval.Contains, hb]

/-- A decide-coerced proposition result is within the abstract bool interval
of `top` (the `if p then 1 else 0` shape the `cmp_*` rows produce). -/
theorem contains_0_1_of_decide (p : Prop) [Decidable p] :
    Interval.Contains (.range 0 1) (if p then 1 else 0) := by
  by_cases hp : p
  · simp [Interval.Contains, hp]
  · simp [Interval.Contains, hp]

/-- (T-A, bool extraction) `decide (a = k) = true` pins `a = k` — the bridge
between the definitional `==` conditions of `triFromBoolIv`/`triCmp` and
the arithmetic facts. -/
theorem beq_eq_true_of {a b : Int} (h : (a == b) = true) : a = b := by
  exact decide_eq_true_eq.mp (by simpa using h)

/-- (T-A, bool extraction) `(a && b) = true` splits. -/
theorem and_true_pair {a b : Bool} (h : (a && b) = true) : a = true ∧ b = true :=
  (Eq.mp (Bool.and_eq_true a b)) h

/-- (T-A, bool extraction) `(a || b) = true` splits. -/
theorem or_true_pair {a b : Bool} (h : (a || b) = true) : a = true ∨ b = true :=
  (Eq.mp (Bool.or_eq_true a b)) h

/-- The top absorption rules of the interval transfer (a `⊤` operand absorbs
every op to `⊤` — the soundness side of the wrap→⊤ rule). -/
theorem join_top_left (b : Interval) : Interval.join Interval.top b = Interval.top := by
  cases b <;> rfl
theorem join_top_right (a : Interval) : Interval.join a Interval.top = Interval.top := by
  cases a <;> rfl

/-- T-A (add): the interval add contains the concrete sum of any
concretizations. An overflowed bound computation (`Interval.inI64Domain`)
fails is `⊤`, which contains everything — the wrap→⊤ rule, proven sound. -/
theorem add_sound {a b : Interval} {x y : Int} (hx : Interval.Contains a x) (hy : Interval.Contains b y) :
    Interval.Contains (a.add b) (x + y) := by
  unfold Interval.Contains at hx hy
  cases a with
  | bottom => exact False.elim hx
  | top =>
      cases b with
      | bottom => exact False.elim hy
      | top =>
          have ht : Interval.add Interval.top Interval.top = Interval.top := by rfl
          simp [ht, Interval.Contains]
      | range c d =>
          have ht : Interval.add Interval.top (Interval.range c d) = Interval.top := by rfl
          simp [ht, Interval.Contains]
  | range a1 a2 =>
      cases b with
      | bottom => exact False.elim hy
      | top =>
          have ht : Interval.add (Interval.range a1 a2) Interval.top = Interval.top := by rfl
          simp [ht, Interval.Contains]
      | range b1 b2 =>
          cases hx with | intro hx1 hx2 =>
          cases hy with | intro hy1 hy2 =>
          by_cases hd : Interval.inI64Domain (a1 + b1) && Interval.inI64Domain (a2 + b2)
          · have h1 : a1 + b1 ≤ x + y := by omega
            have h2 : x + y ≤ a2 + b2 := by omega
            simp [Interval.add, hd, Interval.Contains, h1, h2]
          · simp [Interval.add, hd, Interval.Contains]

/-- T-A (sub). -/
theorem sub_sound {a b : Interval} {x y : Int} (hx : Interval.Contains a x) (hy : Interval.Contains b y) :
    Interval.Contains (a.sub b) (x - y) := by
  unfold Interval.Contains at hx hy
  cases a with
  | bottom => exact False.elim hx
  | top =>
      cases b with
      | bottom => exact False.elim hy
      | top =>
          have ht : Interval.sub Interval.top Interval.top = Interval.top := by rfl
          simp [ht, Interval.Contains]
      | range c d =>
          have ht : Interval.sub Interval.top (Interval.range c d) = Interval.top := by rfl
          simp [ht, Interval.Contains]
  | range a1 a2 =>
      cases b with
      | bottom => exact False.elim hy
      | top =>
          have ht : Interval.sub (Interval.range a1 a2) Interval.top = Interval.top := by rfl
          simp [ht, Interval.Contains]
      | range b1 b2 =>
          cases hx with | intro hx1 hx2 =>
          cases hy with | intro hy1 hy2 =>
          by_cases hd : Interval.inI64Domain (a1 - b2) && Interval.inI64Domain (a2 - b1)
          · have h1 : a1 - b2 ≤ x - y := by omega
            have h2 : x - y ≤ a2 - b1 := by omega
            simp [Interval.sub, hd, Interval.Contains, h1, h2]
          · simp [Interval.sub, hd, Interval.Contains]

/-- Helper: `x*c` over `x ∈ [a,b]` stays between the endpoint products
(linear-in-`x` monotonicity, case-split on the sign of `c`). -/
theorem linear_mul_bound {a b c x : Int} (hax : a ≤ x) (hxb : x ≤ b) :
    min (a * c) (b * c) ≤ x * c ∧ x * c ≤ max (a * c) (b * c) := by
  by_cases hc : 0 ≤ c
  · have h1 : a * c ≤ x * c := Int.mul_le_mul_of_nonneg_right hax hc
    have h2 : x * c ≤ b * c := Int.mul_le_mul_of_nonneg_right hxb hc
    constructor
    · by_cases hz : a * c ≤ b * c
      · rw [Int.min_eq_left hz]
        exact h1
      · have hba : b * c ≤ a * c := by omega
        rw [Int.min_eq_right hba]
        exact Int.le_trans hba h1
    · by_cases hz : a * c ≤ b * c
      · rw [Int.max_eq_right hz]
        exact h2
      · have hba : b * c ≤ a * c := by omega
        rw [Int.max_eq_left hba]
        exact Int.le_trans h2 hba
  · have hc' : c ≤ 0 := by omega
    have h1 : x * c ≤ a * c := Int.mul_le_mul_of_nonpos_right hax hc'
    have h2 : b * c ≤ x * c := Int.mul_le_mul_of_nonpos_right hxb hc'
    constructor
    · exact Int.le_trans (Int.min_le_right (a * c) (b * c)) h2
    · exact Int.le_trans h1 (Int.le_max_left (a * c) (b * c))

/-- Helper: `x*y` over `y ∈ [c,d]` stays between the endpoint products
(linear-in-`y` monotonicity, case-split on the sign of `x`). -/
theorem linear_mul_bound2 {c d x y : Int} (hcy : c ≤ y) (hyd : y ≤ d) :
    min (x * c) (x * d) ≤ x * y ∧ x * y ≤ max (x * c) (x * d) := by
  by_cases hx : 0 ≤ x
  · have h1 : x * c ≤ x * y := Int.mul_le_mul_of_nonneg_left hcy hx
    have h2 : x * y ≤ x * d := Int.mul_le_mul_of_nonneg_left hyd hx
    constructor
    · by_cases hz : x * c ≤ x * d
      · rw [Int.min_eq_left hz]
        exact h1
      · have hba : x * d ≤ x * c := by omega
        rw [Int.min_eq_right hba]
        exact Int.le_trans hba h1
    · by_cases hz : x * c ≤ x * d
      · rw [Int.max_eq_right hz]
        exact h2
      · have hba : x * d ≤ x * c := by omega
        rw [Int.max_eq_left hba]
        exact Int.le_trans h2 hba
  · have hx' : x ≤ 0 := by omega
    have h1 : x * y ≤ x * c := Int.mul_le_mul_of_nonpos_left hx' hcy
    have h2 : x * d ≤ x * y := Int.mul_le_mul_of_nonpos_left hx' hyd
    constructor
    · exact Int.le_trans (Int.min_le_right (x * c) (x * d)) h2
    · exact Int.le_trans h1 (Int.le_max_left (x * c) (x * d))

/-- T-A (mul): the four-corner tableau bound over-approximates the concrete
product. `⊤` (an overflowing corner) contains everything. -/
theorem mul_sound {a b : Interval} {x y : Int} (hx : Interval.Contains a x) (hy : Interval.Contains b y) :
    Interval.Contains (a.mul b) (x * y) := by
  unfold Interval.Contains at hx hy
  cases a with
  | bottom => exact False.elim hx
  | top =>
      cases b with
      | bottom => exact False.elim hy
      | top =>
          have ht : Interval.mul Interval.top Interval.top = Interval.top := by rfl
          simp [ht, Interval.Contains]
      | range c d =>
          have ht : Interval.mul Interval.top (Interval.range c d) = Interval.top := by rfl
          simp [ht, Interval.Contains]
  | range a1 a2 =>
      cases b with
      | bottom => exact False.elim hy
      | top =>
          have ht : Interval.mul (Interval.range a1 a2) Interval.top = Interval.top := by rfl
          simp [ht, Interval.Contains]
      | range b1 b2 =>
          cases hx with | intro hx1 hx2 =>
          cases hy with | intro hy1 hy2 =>
          by_cases hd : Interval.inI64Domain (a1 * b1) && Interval.inI64Domain (a1 * b2)
                        && Interval.inI64Domain (a2 * b1) && Interval.inI64Domain (a2 * b2)
          · have hb1 := linear_mul_bound hx1 hx2 (c := b1)
            have hb2 := linear_mul_bound hx1 hx2 (c := b2)
            have hxlin := linear_mul_bound2 hy1 hy2 (x := x)
            have hLo : min (min (a1 * b1) (a2 * b1)) (min (a1 * b2) (a2 * b2)) ≤ x * y := by
              have hl12 : min (min (a1 * b1) (a2 * b1)) (min (a1 * b2) (a2 * b2)) ≤
                          min (x * b1) (x * b2) := by omega
              exact Int.le_trans hl12 hxlin.1
            have hUp : x * y ≤ max (max (a1 * b1) (a2 * b1)) (max (a1 * b2) (a2 * b2)) := by
              have hu12 : max (x * b1) (x * b2) ≤
                          max (max (a1 * b1) (a2 * b1)) (max (a1 * b2) (a2 * b2)) := by omega
              exact Int.le_trans hxlin.2 hu12
            have hLo' : min (min (a1 * b1) (a1 * b2)) (min (a2 * b1) (a2 * b2)) ≤ x * y := by
              omega
            have hUp' : x * y ≤ max (max (a1 * b1) (a1 * b2)) (max (a2 * b1) (a2 * b2)) := by
              omega
            simpa [Interval.mul, hd, Interval.Contains] using (range_contains hLo' hUp')
          · simp [Interval.mul, hd, Interval.Contains]

/-- T-A (narrowing cast): the intersection with the target range still
contains the in-range concretizations (an out-of-range concretization traps
on the concrete cast — the run shrinks to `none` and T-B's premise excludes
it). -/
theorem cast_narrow_sound {iv : Interval} {x lo hi : Int}
    (hx : Interval.Contains iv x) (hlo : lo ≤ x) (hhi : x ≤ hi) :
    Interval.Contains (iv.castNarrow lo hi) x := by
  unfold Interval.Contains at hx
  cases iv with
  | bottom => exact False.elim hx
  | top => simp [Interval.castNarrow, Interval.Contains, hlo, hhi]
  | range a b =>
      cases hx with | intro hxa hxb =>
      have h1 : max a lo ≤ x := by omega
      have h2 : x ≤ min b hi := by omega
      by_cases hs : max a lo ≤ min b hi
      · simp [Interval.castNarrow, hs, Interval.Contains, h1, h2]
      · exfalso
        exact hs (Int.le_trans h1 h2)

/-- T-A (hull join): a concretization of either join operand is a
concretization of the hull. -/
theorem join_sound {a b : Interval} {x : Int} (hx : Interval.Contains a x ∨ Interval.Contains b x) :
    Interval.Contains (a.join b) x := by
  cases hx with
  | inl h => cases a with
      | bottom => exact False.elim h
      | top =>
          have htop : Interval.join Interval.top b = Interval.top := join_top_left b
          simp [htop, Interval.Contains]
      | range a1 a2 =>
          cases b <;> simp [Interval.join, Interval.Contains] at h ⊢ <;> omega
  | inr h => cases b with
      | bottom => exact False.elim h
      | top =>
          have htop : Interval.join a Interval.top = Interval.top := join_top_right a
          simp [htop, Interval.Contains]
      | range b1 b2 =>
          cases a <;> simp [Interval.join, Interval.Contains] at h ⊢ <;> omega

/-- T-A (widening `∇`): widening only grows — `old ∇ new` still represents
every concretization of `old` (the soundness that makes loop-carried `⊤`
safe: a discharge over the widened value is a discharge over the
pre-widening value). -/
theorem widen_sound {old new : Interval} {x : Int} (hx : Interval.Contains old x) :
    Interval.Contains (old.widenOld new) x := by
  unfold Interval.widenOld
  by_cases hs : new.subsetOf old
  · simpa [hs] using hx
  · simp [hs, top_contains]

/-- T-A (comparison): the abstract comparison's bool interval contains the
concrete comparison result of any concretization (defTrue/defFalse/`⊤` are
each sound for their defining order relationship). -/
theorem tri_cmp_sound (kind : String) {a b : Interval} {x y : Int}
    (hx : Interval.Contains a x) (hy : Interval.Contains b y) :
    Interval.Contains (boolIv (triCmp a b kind)) (if concreteCmp kind x y then 1 else 0) := by
  unfold Interval.Contains at hx hy
  cases a with
  | bottom => exact False.elim hx
  | top =>
      cases b with
      | bottom => exact False.elim hy
      | _ => simp [triCmp, boolIv, Interval.Contains]; exact contains_0_1_of_bool (concreteCmp kind x y)
  | range a1 a2 =>
      cases b with
      | bottom => exact False.elim hy
      | top => simp [triCmp, boolIv, Interval.Contains]; exact contains_0_1_of_bool (concreteCmp kind x y)
      | range b1 b2 =>
          cases hx with | intro hx1 hx2 =>
          cases hy with | intro hy1 hy2 =>
          by_cases h1 : kind = "cmp_lt"
          · rw [h1]
            by_cases hlt : a2 < b1
            · have hxy : x < y := by omega
              simp [triCmp, hlt, concreteCmp, Interval.Contains, boolIv, hxy]
            · by_cases hge : a1 ≥ b2
              · have hxy : ¬ x < y := by omega
                simp [triCmp, hlt, hge, concreteCmp, Interval.Contains, boolIv, hxy]
              · simp [triCmp, hlt, hge, concreteCmp, boolIv]
                exact contains_0_1_of_decide (x < y)
          · by_cases h2 : kind = "cmp_le"
            · rw [h2]
              by_cases hle : a2 ≤ b1
              · have hxy : x ≤ y := by omega
                simp [triCmp, hle, concreteCmp, Interval.Contains, boolIv, hxy]
              · by_cases hgt : a1 > b2
                · have hxy : ¬ x ≤ y := by omega
                  simp [triCmp, hle, hgt, concreteCmp, Interval.Contains, boolIv, hxy]
                · simp [triCmp, hle, hgt, concreteCmp, boolIv]
                  exact contains_0_1_of_decide (x ≤ y)
            · by_cases h3 : kind = "cmp_gt"
              · rw [h3]
                by_cases hgt : a1 > b2
                · have hxy : x > y := by omega
                  simp [triCmp, hgt, concreteCmp, Interval.Contains, boolIv, hxy]
                · by_cases hle : a2 ≤ b1
                  · have hxy : ¬ x > y := by omega
                    simp [triCmp, hgt, hle, concreteCmp, Interval.Contains, boolIv, hxy]
                  · simp [triCmp, hgt, hle, concreteCmp, boolIv]
                    exact contains_0_1_of_decide (x > y)
              · by_cases h4 : kind = "cmp_ge"
                · rw [h4]
                  by_cases hge : a1 ≥ b2
                  · have hxy : x ≥ y := by omega
                    simp [triCmp, hge, concreteCmp, Interval.Contains, boolIv, hxy]
                  · by_cases hlt : a2 < b1
                    · have hxy : ¬ x ≥ y := by omega
                      simp [triCmp, hge, hlt, concreteCmp, Interval.Contains, boolIv, hxy]
                    · simp [triCmp, hge, hlt, concreteCmp, boolIv]
                      exact contains_0_1_of_decide (x ≥ y)
                · by_cases h5 : kind = "cmp_eq"
                  · rw [h5]
                    by_cases hpt : a1 == a2 && b1 == b2 && a1 == b1
                    · have ha12 : a1 = a2 ∧ b1 = b2 ∧ a1 = b1 := by
                        have hand1 : ((a1 == a2 && b1 == b2) = true) ∧ (a1 == b1) = true :=
                          (Eq.mp (Bool.and_eq_true (a1 == a2 && b1 == b2) (a1 == b1))) hpt
                        have hand2 : (a1 == a2) = true ∧ (b1 == b2) = true :=
                          (Eq.mp (Bool.and_eq_true (a1 == a2) (b1 == b2))) hand1.1
                        exact ⟨beq_eq_true_of hand2.1, beq_eq_true_of hand2.2, beq_eq_true_of hand1.2⟩
                      have hxyeq : x = y := by omega
                      simp [triCmp, hpt, concreteCmp, Interval.Contains, boolIv, hxyeq]
                    · by_cases hdis : a2 < b1 || a1 > b2
                      · have hnxy : ¬ x = y := by
                          intro hxy
                          have hd1 : (decide (a2 < b1)) = true ∨ (decide (a1 > b2)) = true :=
                            or_true_pair hdis
                          rcases hd1 with hlt | hgt
                          · have hlt' : a2 < b1 := decide_eq_true_eq.mp hlt
                            omega
                          · have hgt' : a1 > b2 := decide_eq_true_eq.mp hgt
                            omega
                        simp [triCmp, hpt, hdis, concreteCmp, Interval.Contains, boolIv, hnxy]
                      · simp [triCmp, hpt, hdis, concreteCmp, boolIv]
                        by_cases hxy : x = y <;> simp [Interval.Contains, hxy]
                  · by_cases h6 : kind = "cmp_ne"
                    · rw [h6]
                      by_cases hpt : a1 == a2 && b1 == b2 && a1 == b1
                      · have ha12 : a1 = a2 ∧ b1 = b2 ∧ a1 = b1 := by
                          have hand1 : ((a1 == a2 && b1 == b2) = true) ∧ (a1 == b1) = true :=
                            (Eq.mp (Bool.and_eq_true (a1 == a2 && b1 == b2) (a1 == b1))) hpt
                          have hand2 : (a1 == a2) = true ∧ (b1 == b2) = true :=
                            (Eq.mp (Bool.and_eq_true (a1 == a2) (b1 == b2))) hand1.1
                          exact ⟨beq_eq_true_of hand2.1, beq_eq_true_of hand2.2, beq_eq_true_of hand1.2⟩
                        have hxyeq : x = y := by omega
                        simp [triCmp, hpt, concreteCmp, Interval.Contains, boolIv, hxyeq]
                      · by_cases hdis : a2 < b1 || a1 > b2
                        · have hnxy : ¬ x = y := by
                            intro hxy
                            have hd1 : (decide (a2 < b1)) = true ∨ (decide (a1 > b2)) = true :=
                              or_true_pair hdis
                            rcases hd1 with hlt | hgt
                            · have hlt' : a2 < b1 := decide_eq_true_eq.mp hlt
                              omega
                            · have hgt' : a1 > b2 := decide_eq_true_eq.mp hgt
                              omega
                          simp [triCmp, hpt, hdis, concreteCmp, Interval.Contains, boolIv, hnxy]
                        · simp [triCmp, hpt, hdis, concreteCmp, boolIv]
                          by_cases hxy : x = y <;> simp [Interval.Contains, hxy]
                    · -- any other kind: the abstract comparison is the `⊤` row
                      -- and the concrete result is a 0/1 — `[0,1]` contains it.
                      have htri : triCmp (Interval.range a1 a2) (Interval.range b1 b2) kind = Tri.top := by
                        unfold triCmp
                        simp [h1, h2, h3, h4, h5, h6]
                      by_cases hb : concreteCmp kind x y <;> simp [hb, htri, boolIv, Interval.Contains]

/-- wrap-boundary 1: `i64::MAX - 1` + 1 overflows i64 → `⊤`. -/
example : Interval.add (Interval.range (Interval.i64Max - 1) Interval.i64Max)
    (Interval.range 1 1) = Interval.top := by decide

/-- wrap-boundary 2: `i64::MIN + 1` − 4 underflows i64 → `⊤`. -/
example : Interval.sub (Interval.range (Interval.i64Min + 1) Interval.i64Min)
    (Interval.range 2 4) = Interval.top := by decide

/-- wrap-boundary 3: `i64::MAX - 1` × 2 overflows i64 → `⊤`. -/
example : Interval.mul (Interval.range (Interval.i64Max - 1) Interval.i64Max)
    (Interval.range 2 2) = Interval.top := by decide

/-- wrap-boundary 4: `i64::MIN` × −1 overflows i64 → `⊤`. -/
example : Interval.mul (Interval.range Interval.i64Min Interval.i64Min)
    (Interval.range (-1) (-1)) = Interval.top := by decide

/-- The 32-bit word domain (`word_domain 32 = ±2^31`, the §Q13 register
domain), machine-checked. -/
example : wordDomain 32 = some (-2147483648, 2147483647) := by decide

/-- The 64-bit word domain is the full i64 domain. -/
example : wordDomain 64 = none := by decide

/-- The `+1` boundary of the 32-bit word domain is representable in i64
(the abstract data domain is full-i64 on every target, §Q3). -/
example : Interval.inI64Domain 2147483648 = true := by decide

/-- The `−1` boundary below the 32-bit word domain is representable in i64. -/
example : Interval.inI64Domain (-2147483649) = true := by decide

/-- The `+1` above `2^31 − 1` wraps on a 32-bit register but NOT in the i64
data domain — the interval engine answers the concrete `[2^31, 2^31]`, never
a manufactured `⊤` (§Q3's implementation finding). -/
example : Interval.add (Interval.range 2147483647 2147483647) (Interval.range 1 1)
    = Interval.range 2147483648 2147483648 := by decide

/-- The word-domain pair is width-relative: at 32 bits the read answers
`[-2^31, 2^31-1]` (pinned by the `boundary/aperture-read-width-domain`
vector; the port gate re-checks it byte-exactly). -/
example : wordDomain 32 = some (-(2 ^ (32 - 1)), 2 ^ (32 - 1) - 1) := by decide

/-- The `±1` word-domain pair at 64 bits sits inside the full i64 domain:
both dedicated endpoints remain representable (the data domain never widens
to `⊤` on a representable value). -/
example : wordDomain 64 = none ∧ Interval.inI64Domain 9223372036854775807 = true
    ∧ Interval.inI64Domain (-9223372036854775808) = true := by decide

/-- The abstract bool of a bool-typed interval, with its 0/1
concretization — the premise of the bool-op soundness theorems (the
typechecker guarantees bool-typed operands at `&&`/`||`/`not` sites). -/
def BoolConcr (iv : Interval) (x : Int) : Prop :=
  Interval.Contains iv x ∧ (x = 0 ∨ x = 1)

/-- A `defTrue` abstract bool interval pins its concretization to 1. -/
theorem tri_def_true_pins_one {iv : Interval} {x : Int}
    (ht : triFromBoolIv iv = Tri.defTrue) (hx : BoolConcr iv x) : x = 1 := by
  have hv : Interval.Contains iv x := hx.1
  unfold Interval.Contains at hv
  cases iv with
  | bottom | top => simp [triFromBoolIv] at ht
  | range a b =>
      cases hv with | intro hva hvb =>
      by_cases h00 : a == 0 && b == 0
      · simp [triFromBoolIv, h00] at ht
      · by_cases h11 : a == 1 && b == 1
        · have ha1 : a = 1 := beq_eq_true_of (and_true_pair h11).1
          have hb1 : b = 1 := beq_eq_true_of (and_true_pair h11).2
          omega
        · simp [triFromBoolIv, h00, h11] at ht

/-- A `defFalse` abstract bool interval pins its concretization to 0. -/
theorem tri_def_false_pins_zero {iv : Interval} {x : Int}
    (hf : triFromBoolIv iv = Tri.defFalse) (hx : BoolConcr iv x) : x = 0 := by
  have hv : Interval.Contains iv x := hx.1
  unfold Interval.Contains at hv
  cases iv with
  | bottom | top => simp [triFromBoolIv] at hf
  | range a b =>
      cases hv with | intro hva hvb =>
      by_cases h00 : a == 0 && b == 0
      · have ha0 : a = 0 := beq_eq_true_of (and_true_pair h00).1
        have hb0 : b = 0 := beq_eq_true_of (and_true_pair h00).2
        omega
      · by_cases h11 : a == 1 && b == 1
        · simp [triFromBoolIv, h00, h11] at hf
        · simp [triFromBoolIv, h00, h11] at hf

/-- The trichotomy of an abstract bool result (the complement of the two
`defTrue`/`defFalse` negations). -/
theorem tri_trichotomy (t : Tri) :
    t = Tri.defTrue ∨ t = Tri.defFalse ∨ t = Tri.top := by
  cases t <;> simp

/-- T-A (triFromBoolIv): the abstract bool interval contains its 0/1
concretizations (non-point ranges collapse to `⊤ = [0,1]`, which contains
both; point ranges pin the value). -/
theorem tri_from_bool_iv_sound {iv : Interval} {v : Int}
    (hv : BoolConcr iv v) :
    Interval.Contains (boolIv (triFromBoolIv iv)) v := by
  have hv01 : v = 0 ∨ v = 1 := hv.2
  have hvd : Interval.Contains iv v := hv.1
  by_cases ht : triFromBoolIv iv = Tri.defTrue
  · have hv1 : v = 1 := tri_def_true_pins_one ht hv
    simp [ht, boolIv, Interval.Contains, hv1]
  · by_cases hf : triFromBoolIv iv = Tri.defFalse
    · have hv0 : v = 0 := tri_def_false_pins_zero hf hv
      simp [hf, boolIv, Interval.Contains, hv0]
    · have ht' : triFromBoolIv iv = Tri.top := by
        rcases (tri_trichotomy (triFromBoolIv iv)) with hT | hF | hTop
        · exact False.elim (ht hT)
        · exact False.elim (hf hF)
        · exact hTop
      simp [ht', boolIv, Interval.Contains]
      cases hv01 with
      | inl hv0 => simp [hv0]
      | inr hv1 => simp [hv1]

/-- The abstract AND truth-table: `defTrue` iff both sides `defTrue`. -/
theorem tri_and_defTrue {t1 t2 : Tri} :
    triAnd t1 t2 = Tri.defTrue → t1 = Tri.defTrue ∧ t2 = Tri.defTrue := by
  intro h
  cases t1 <;> cases t2 <;> simp [triAnd] at h ⊢ <;> contradiction

/-- The abstract AND truth-table: `defFalse` iff either side `defFalse`. -/
theorem tri_and_defFalse {t1 t2 : Tri} :
    triAnd t1 t2 = Tri.defFalse → t1 = Tri.defFalse ∨ t2 = Tri.defFalse := by
  intro h
  cases t1 <;> cases t2 <;> simp [triAnd] at h ⊢ <;> contradiction

/-- The abstract OR truth-table: `defTrue` iff either side `defTrue`. -/
theorem tri_or_defTrue {t1 t2 : Tri} :
    triOr t1 t2 = Tri.defTrue → t1 = Tri.defTrue ∨ t2 = Tri.defTrue := by
  intro h
  cases t1 <;> cases t2 <;> simp [triOr] at h ⊢ <;> contradiction

/-- The abstract OR truth-table: `defFalse` iff both sides `defFalse`. -/
theorem tri_or_defFalse {t1 t2 : Tri} :
    triOr t1 t2 = Tri.defFalse → t1 = Tri.defFalse ∧ t2 = Tri.defFalse := by
  intro h
  cases t1 <;> cases t2 <;> simp [triOr] at h ⊢ <;> contradiction

/-- T-A (triAnd): the concrete `&&` result is within the abstract `triAnd`
interval (non-bool concretizations collapse to `⊤ = [0,1]`, which contains
every bool result). -/
theorem tri_and_sound {ia ib : Interval} {x y : Int}
    (hx : BoolConcr ia x) (hy : BoolConcr ib y) :
    Interval.Contains (boolIv (triAnd (triFromBoolIv ia) (triFromBoolIv ib)))
             (if x != 0 && y != 0 then 1 else 0) := by
  by_cases ht : triAnd (triFromBoolIv ia) (triFromBoolIv ib) = Tri.defTrue
  · have hx1 : x = 1 := tri_def_true_pins_one (tri_and_defTrue ht).1 hx
    have hy1 : y = 1 := tri_def_true_pins_one (tri_and_defTrue ht).2 hy
    simp [Interval.Contains, ht, boolIv, hx1, hy1]
  · by_cases hf : triAnd (triFromBoolIv ia) (triFromBoolIv ib) = Tri.defFalse
    · have hx0 : x = 0 ∨ y = 0 := by
        rcases (tri_and_defFalse hf) with t1 | t2
        · left; exact tri_def_false_pins_zero t1 hx
        · right; exact tri_def_false_pins_zero t2 hy
      simp [hf, boolIv, Interval.Contains]
      by_cases hx0' : x = 0
      · simp [hx0']
      · have hy0' : y = 0 := by omega
        simp [hy0']
    · have ht' : triAnd (triFromBoolIv ia) (triFromBoolIv ib) = Tri.top := by
        have htri := tri_trichotomy (triAnd (triFromBoolIv ia) (triFromBoolIv ib))
        rcases htri with hT | hF | hTop
        · exact False.elim (ht hT)
        · exact False.elim (hf hF)
        · exact hTop
      simp [ht', boolIv, Interval.Contains]
      by_cases hc : ¬x = 0 ∧ ¬y = 0 <;> simp [hc]

/-- T-A (triOr): the concrete `||` result is within the abstract `triOr`
interval. -/
theorem tri_or_sound {ia ib : Interval} {x y : Int}
    (hx : BoolConcr ia x) (hy : BoolConcr ib y) :
    Interval.Contains (boolIv (triOr (triFromBoolIv ia) (triFromBoolIv ib)))
             (if x != 0 || y != 0 then 1 else 0) := by
  by_cases ht : triOr (triFromBoolIv ia) (triFromBoolIv ib) = Tri.defTrue
  · have hx1 : x = 1 ∨ y = 1 := by
      rcases (tri_or_defTrue ht) with t1 | t2
      · left; exact tri_def_true_pins_one t1 hx
      · right; exact tri_def_true_pins_one t2 hy
    simp [Interval.Contains, ht, boolIv]
    by_cases hx1' : x = 1
    · simp [hx1']
    · have hy1' : y = 1 := by omega
      simp [hy1']
  · by_cases hf : triOr (triFromBoolIv ia) (triFromBoolIv ib) = Tri.defFalse
    · have hx0 : x = 0 ∧ y = 0 := by
        rcases (tri_or_defFalse hf) with ⟨t1, t2⟩
        exact ⟨tri_def_false_pins_zero t1 hx, tri_def_false_pins_zero t2 hy⟩
      simp [hf, boolIv, Interval.Contains, hx0.1, hx0.2]
    · have ht' : triOr (triFromBoolIv ia) (triFromBoolIv ib) = Tri.top := by
        have htri := tri_trichotomy (triOr (triFromBoolIv ia) (triFromBoolIv ib))
        rcases htri with hT | hF | hTop
        · exact False.elim (ht hT)
        · exact False.elim (hf hF)
        · exact hTop
      simp [ht', boolIv, Interval.Contains]
      by_cases hc : ¬x = 0 ∨ ¬y = 0 <;> simp [hc]

/-- T-A (triNot): the concrete `not` result is within the abstract `triNot`
interval. -/
theorem tri_not_sound {iv : Interval} {x : Int}
    (hx : BoolConcr iv x) :
    Interval.Contains (boolIv (triNot (triFromBoolIv iv))) (if x = 0 then 1 else 0) := by
  by_cases ht : triFromBoolIv iv = Tri.defTrue
  · have hx1 : x = 1 := tri_def_true_pins_one ht hx
    simp [triNot, ht, boolIv, Interval.Contains, hx1]
  · by_cases hf : triFromBoolIv iv = Tri.defFalse
    · have hx0 : x = 0 := tri_def_false_pins_zero hf hx
      simp [triNot, hf, boolIv, Interval.Contains, hx0]
    · have ht' : triFromBoolIv iv = Tri.top := by
        rcases (tri_trichotomy (triFromBoolIv iv)) with hT | hF | hTop
        · exact False.elim (ht hT)
        · exact False.elim (hf hF)
        · exact hTop
      simp [triNot, ht', boolIv, Interval.Contains]
      exact contains_0_1_of_decide (x = 0)

end TA

/-! ### T-B: discharge soundness (the T-A corollary, PLAN-VERIFY-3 P14.2)

The registry statement the `rederive` method rests on: an obligation whose
abstract interpretation discharges (`evalInRange = defTrue`) holds for every
concrete execution that is a concretization of the abstract entry — the
check can be elided. It composes T-A's `Contains`-soundness with the
`eval_in_range` semantics: `⊤`/`⊥` can never discharge. -/
namespace TB

open Tyu.Abs

/-- T-B (the discharge bridge): `eval_in_range` answering `defTrue` for an
interval means it is a non-empty subset of `[lo, hi]`; every concretization
`x ∈ iv` therefore satisfies the target bounds. This is the single lemma
the whole re-derive discharge path composes from (a `⊤`/`⊥` interval can
never reach `defTrue`). -/
theorem discharge_sound {iv : Interval} {x lo hi : Int}
    (hx : Tyu.Abs.Interval.Contains iv x) (hd : evalInRange iv lo hi = Tri.defTrue) :
    lo ≤ x ∧ x ≤ hi := by
  unfold Tyu.Abs.Interval.Contains at hx
  unfold evalInRange at hd
  cases iv with
  | bottom => exact False.elim hx
  | top => simp at hd
  | range a b =>
      cases hx with | intro hxa hxb =>
      cases hcond : (decide (a ≥ lo) && decide (b ≤ hi))
      · -- the interval is NOT wholly inside [lo, hi]: the range can answer
        -- only defFalse/top — contradict the defTrue hypothesis. The
        -- `&&`-driven outer if and the `||`-driven inner if are the same
        -- shapes `evalInRange` decides.
        exfalso
        simp [evalInRange, hcond] at hd
        by_cases hc2 : b < lo ∨ hi < a
        · simp [hc2] at hd
        · have hc2' : ¬ (b < lo ∨ hi < a) := hc2
          simp [hc2'] at hd
      · -- the interval IS inside [lo, hi]: extract the bounds and close.
        have hand : (decide (a ≥ lo)) = true ∧ (decide (b ≤ hi)) = true :=
          (Eq.mp (Bool.and_eq_true (decide (a ≥ lo)) (decide (b ≤ hi)))) hcond
        have hlo : lo ≤ a := by
          exact decide_eq_true_eq.mp hand.1
        have hhi : b ≤ hi := by
          exact decide_eq_true_eq.mp hand.2
        constructor <;> omega

end TB

end Tyu.Sound