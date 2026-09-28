import Tyu.Step
import Tyu.Src
import Tyu.Bundles

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
`armv7m`, `riscv32` — each with the RAM window declared by its
`model/model.toml [memory] ram`):

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
  have hd : Tyu.Conformance.wordDomain w =
      some (-((2 : Int) ^ (w - 1)), ((2 : Int) ^ (w - 1)) - 1) := by
    simp [Tyu.Conformance.wordDomain, hb, hw64]
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
  have hd : Tyu.Conformance.wordDomain w = none := by
    simp [Tyu.Conformance.wordDomain, hb, hw64]
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
    Tyu.Conformance.MemModel.apertureRead (Tyu.Conformance.MemModel.bundle lo hi cells) w
      = Tyu.Bundles.toConformance (BundleMem.apertureRead w) :=
  Tyu.Bundles.aperture_read_agrees lo hi cells w

/-- T-D (mirror equivalence, load): a point-address load of the recorded
cells reads identically under both mirrors. -/
theorem bundle_load_agrees (lo hi : Int) (cells : List (Int × Tyu.Conformance.Interval)) (a : Int)
    (w : Nat) :
    Tyu.Conformance.MemModel.load (Tyu.Conformance.MemModel.bundle lo hi cells)
        (Tyu.Conformance.Interval.range a a) w
      = Tyu.Bundles.toConformance (BundleMem.loadVal (ofCells lo hi cells) (IntervalVal.range a a)) :=
  Tyu.Bundles.load_agrees lo hi cells a w

end TD

end Tyu.Sound
