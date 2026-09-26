import Tyu.Step

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

end Tyu.Sound