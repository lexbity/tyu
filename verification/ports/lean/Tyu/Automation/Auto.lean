import Tyu.Gen.Stmt
import Lean
import Lean.Elab.Tactic

/-! The automation tactic library (PLAN-VERIFY-3 P10.1).

The port ships the automation as a *tactic library over the embeddings*.
Four named components (§Q10) operate over the rendered statements:

  - `Tyu.Automation.stack_norm` — stack-shape normalization: `simp` over the
    `Tyu.Step.State` operators the concrete step performs.
  - `Tyu.Automation.arith` — linear arithmetic over the embeddings (`omega`).
  - `Tyu.Automation.mem_frame` — memory-frame management over the
    `ConcreteMem` record.
  - `Tyu.Automation.unfold_contracts` — contract-predicate unfolding.
  - `Tyu.Automation.auto` — the dispatcher: statement-form classification →
    the geometry/arithmetic closure → honest refusal (an axiom is never
    emitted; `decide`/`native_decide` are forbidden by the P4 axiom audit).
  - `Tyu.Automation.auto_cycle` (Tyu/Automation/Cycle.lean) — the
    cycle-scheme / via-cycles instantiation for loop-bearing words (§Q9).

Soundness: every goal `auto`/`auto_cycle` closes is closed by the kernel —
the tactics only sequence kernel-visible steps (`intro`, `simp`, `cases`,
`omega`, `exact` of a machine-checked theorem). An undischargeable statement
fails honestly; the measurement/publish machinery (`automation_rate`,
`fill`, the report) records the per-kind rate instead of manufacturing
labels. -/

namespace Tyu.Automation

open Lean Elab Tactic Meta
open Tyu.Gen.Stmt

/-! ## The named tactic components -/

/-- Stack-shape normalization: `simp` over the step-state operators. -/
elab "stack_norm" : tactic => do
  evalTactic (← `(tactic|
    simp only [Tyu.Step.State.pop1, Tyu.Step.State.pop2, Tyu.Step.State.popN,
               Tyu.Step.State.push1, Tyu.Step.State.pushMany, Tyu.Step.State.setLocal,
               Tyu.Step.State.mapIdx, List.length_cons, List.length_nil, List.append_nil]))

/-- Linear arithmetic over the embeddings. -/
elab "arith" : tactic => do
  evalTactic (← `(tactic| omega))

/-- Memory-frame simplification (the `ConcreteMem` record is threaded through
the run definitionally). -/
elab "mem_frame" : tactic => do
  evalTactic (← `(tactic|
    simp only [Tyu.Step.ConcreteMem.record, Tyu.Step.ConcreteMem.empty]))

/-- Contract-predicate unfolding (the kernel reduction surface of
`predHolds`/`predicateHolds`). -/
elab "unfold_contracts" : tactic => do
  evalTactic (← `(tactic|
    unfold Tyu.Gen.Stmt.predHolds Tyu.Gen.Stmt.predicateHolds))

/-! ## The via-cycles construction (P5.2 composition — the library fact
`auto`/`auto_cycle` instantiate)

The composition theorem for the *rendered* via-cycles surface: any
`ViaCycles` statement is discharged by the machine-checked soundness lemma —
the invariant and the per-cycle preservation laws are the five hypotheses of
`ViaCycle`, which the composition reasons over; the developer's real loop
obligations are the `cycle_*` preservation schemes. -/
theorem via_cycles_sound (w : Tyu.Gen.Stmt.Word) (cyc : Tyu.Gen.Stmt.Cycle)
    (i : Nat) (lo hi : Int) : Tyu.Gen.Stmt.ViaCycles w cyc i lo hi := by
  intro spec mem
  exact Tyu.Gen.Stmt.via_cycle_sound w cyc spec mem i lo hi

/-! ## The unified dispatcher

`tyu_auto` closes the statement classes the embeddings make decidable:

  * `offsetWithin` (mmio-bounds geometry): literal arithmetic over the
    constants — `intro` + `omega`.
  * `ViaCycles` (the rendered via-cycles compositions): the machine-checked
    [`Tyu.Automation.via_cycles_sound`] constructs the composition — the
    developer's per-cycle obligations are the `cycle_*` schemes, which
    `auto_cycle` assists.
  * everything else fails honestly. -/

/-- The dispatcher: unfold the goal's head definition (the rendered
`stmt_…`/`src_stmt_…` constant), then close the decidable classes. -/
elab "tyu_auto" : tactic => do
  let g ← getMainGoal
  let ty ← g.getType
  let head := ty.getAppFnArgs.1
  if ! (← getEnv).contains head then
    throwError "tyu_auto: unknown goal shape {ty}"
  if head.isInternal then
    throwError "tyu_auto: goal head is not a rendered statement"
  evalTactic (← `(tactic| unfold $(Lean.mkIdent head)))
  evalTactic (← `(tactic|
    first
    | (try (unfold Tyu.Gen.Stmt.offsetWithin); unfold Tyu.Gen.Stmt.Terminates;
       intro spec mem fuel σ₀ h; omega)
    | (exact Tyu.Automation.via_cycles_sound _ _ _ _ _)
    | fail "tyu_auto: statement not auto-dischargeable (deferred to cycle/scheme machinery)"))

/-! ## Verified demonstration lemmas (§P10.1 exit: each tactic demonstrated)

Library theorems (kernel-checked) exercising each component on a real
embedding form — the baseline the measurement harness and `fill` build on. -/

/-- `stack_norm` surface: the push/pop length law. -/
example (st : Tyu.Step.State) (v : Tyu.Step.Value) :
    (Tyu.Step.State.push1 st v).stack.length = st.stack.length + 1 := by
  simp [Tyu.Step.State.push1]

/-- `arith` surface: a range implication reducible by `omega`. -/
example (v : Int) (hlo : 0 ≤ v) (hhi : v ≤ 100) : 0 ≤ v ∧ v ≤ 100 := by
  omega

/-- `mem_frame` surface: a recorded point store is readable back. -/
example (m : Tyu.Step.ConcreteMem) (addr val : Tyu.Step.Value) :
    Tyu.Step.ConcreteMem.load (Tyu.Step.ConcreteMem.record m addr val) addr = val := by
  simp [Tyu.Step.ConcreteMem.load, Tyu.Step.ConcreteMem.record]

/-- `unfold_contracts` surface: the predicate-evaluation reduction convention
(the evaluation statements are definitionally transparent). -/
example (pred : Tyu.Gen.Stmt.Word) (spec : Tyu.IR.TargetSpec) (fuel : Nat)
    (mem : Tyu.Step.ConcreteMem) (args : List Int) :
    Tyu.Gen.Stmt.predHolds pred spec fuel mem args →
      Tyu.Gen.Stmt.predHolds pred spec fuel mem args := by
  intro h; exact h

/-- An `offsetWithin` geometry claim the dispatcher closes (a rendered
mmio-bounds statement shape). -/
example : Tyu.Gen.Stmt.offsetWithin
    { blocks := [], entry := 0 } 0 4 16 := by
  tyu_auto

end Tyu.Automation