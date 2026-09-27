import Tyu.Automation.Auto
import Tyu.Gen.Stmt
import Lean

/-! Cycle-lemma instantiation: `Tyu.Automation.auto_cycle` (PLAN-VERIFY-3
P10.2).

Loop-bearing words render per-cycle *schemes* (`cycle_<word>_<n>`, §Q9) —
universally-quantified preservation laws over the developer's invariant — and
the *via-cycles compositions* (`stmt_…_via_cycles`, §P5.2), which the
machine-checked [`Tyu.Automation.via_cycles_sound`] discharges. `auto_cycle`
instantiates and assists:

  * on a `ViaCycles`-shaped goal it applies `via_cycles_sound` (the
    composition is a theorem, not a marker);
  * on a `cycle_*` preservation scheme it unfolds the member-step relation
    (`StepTo`/`StepOut`) and reduces the block-local arithmetic — a residue
    is an honest open obligation, not a fabricated discharge.

Soundness: everything is kernel-gated; the axiom audit (P7) applies to the
resulting theorems unchanged. -/

namespace Tyu.Automation

open Lean Elab Tactic Meta
open Tyu.Gen.Stmt

/-- `auto_cycle`: unfold the goal head (the rendered via/scheme constant),
then instantiate/discharge. -/
elab "auto_cycle" : tactic => do
  let g ← getMainGoal
  let ty ← g.getType
  let head := ty.getAppFnArgs.1
  if ! (← getEnv).contains head then
    throwError "auto_cycle: unknown goal shape {ty}"
  evalTactic (← `(tactic| unfold $(Lean.mkIdent head)))
  evalTactic (← `(tactic|
    first
    | (exact Tyu.Automation.via_cycles_sound _ _ _ _ _)
    | (unfold Tyu.Gen.Stmt.StepTo Tyu.Gen.Stmt.StepOut;
       intro spec mem Inv b t σ σ' hm1 hm2 hinv hstep;
       simp_all)
    | fail "auto_cycle: not a via-cycles or cycle-scheme goal"))

/-! ## Template invariants (§Q10: range-carrying, accumulator, monotone)

The invariant the developer instantiates a cycle scheme with is drawn from a
template family over the *exit-observation surface* — the same projections
the rendered statements use. `auto_cycle` instantiates these; the
demonstrations below show the §11.1 mechanic: a cycle lemma that *fills from
a range template*. -/

/-- Range-carrying invariant: the i-th output stays in `[lo, hi]`. -/
def rangeCarryingInv (i : Nat) (lo hi : Int) (σ : Tyu.Step.State) : Prop :=
  lo ≤ Tyu.Gen.Stmt.outputAt σ i ∧ Tyu.Gen.Stmt.outputAt σ i ≤ hi

/-- Accumulator-bounded invariant: the i-th output never exceeds a bound. -/
def accumBoundInv (i : Nat) (b : Int) (σ : Tyu.Step.State) : Prop :=
  Tyu.Gen.Stmt.outputAt σ i ≤ b

/-- Monotone-counter invariant: the i-th output is bounded below (a counter
that only grows). -/
def monotoneInv (i : Nat) (lo : Int) (σ : Tyu.Step.State) : Prop :=
  lo ≤ Tyu.Gen.Stmt.outputAt σ i

/-- A cycle scheme *filled from the range template* (§11.1 observation:
the developer instantiates `Inv` with `rangeCarryingInv`): for a cycle
with NO member-to-member steps, the preservation scheme is vacuous and the
template instantiation closes it — `auto_cycle`'s scheme-side branch
discharges the same shape. -/
example (w : Tyu.Gen.Stmt.Word) (cyc : Tyu.Gen.Stmt.Cycle)
    (hNoMembers : cyc.members = []) :
    ∀ (spec : Tyu.IR.TargetSpec) (mem : Tyu.Step.ConcreteMem)
      (Inv : Tyu.Step.State → Prop) (b t : Nat) (σ σ' : Tyu.Step.State),
      Tyu.Gen.Stmt.Member cyc b → Tyu.Gen.Stmt.Member cyc t → Inv σ →
        Tyu.Gen.Stmt.StepTo w cyc spec mem b t σ σ' → Inv σ' := by
  intro spec mem Inv b t σ σ' hb ht hinv hstep
  exfalso
  simp [Tyu.Gen.Stmt.Member, Tyu.Gen.Stmt.memberOf, hNoMembers] at hb

/-- The template surfaced as a usable invariant: the range template bounds
an exit observation (the projection the rendered statements use). -/
example (σ : Tyu.Step.State) (i : Nat) (lo hi : Int) :
    rangeCarryingInv i lo hi σ → lo ≤ Tyu.Gen.Stmt.outputAt σ i := by
  intro h; exact h.1

end Tyu.Automation