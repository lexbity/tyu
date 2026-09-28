import Gen.Uart7

/-
The P13.2 refined worked example, proven at the SOURCE surface:
`Uart7::read-tx-idle::subtype-range::1` is a theorem of the generated
`src_stmt_Uart7_read_tx_idle_subtype_range_1` (`Tyu.Src.outInRange` over the
pure-fragment word) — the harvest certifies it with `surface: "source"`,
`proof.relies: ["T-S"]`, and the statement hash carrying
`refinement: "rp2350.uart-fr"` in its context (§Q13/P13.1: the claim is
relativized to the rp2350 bundle's UARTFR device refinement).

The word is `vol_load u32 uart.UARTFR; drop u32; const_i64 1; ret` — a
single-block PURE-FRAGMENT word (every op is in `Tyu.Src.Op`): it reads the
refined register, discards the read, and pushes the constant 1. Its
statement is `src_stmt_… : outInRange blocks 0 0 0 1` — every terminating
source run's single output is a subtype-valid bit, whatever the refined
UARTFR register read answered (read-dropped; the §Q13 universal
quantification makes the claim true over every oracle). The proof evaluates
the fragment run (a `simp`-closed run computation over `Tyu.Src`), observes
the exit value is the pushed constant, and closes the range by arithmetic —
exactly the SumFix (P9.2) pattern.

Note on the fragment boundary (recorded, `REVIEW.md` §4): the fixture's
word IR is fragment-parseable BECAUSE it omits the `addr_of`/`mmio_place`
address materialization that langc's lowering inserts ahead of a real MMIO
`vol_load` (those ops are NOT in `Tyu.Src.Op`, so real-MIO words are
refused at the source surface — `non-fragment-ops`). The refined example
therefore demonstrates the source-surface mechanism on the fragment word;
the address-materialization boundary stays the honest refusal for full
lowerings.

No proof placeholders: the axiom audit gates the theorem's axiom closure. -/

namespace Tyu.Gen.Corpus.Uart7

open Tyu.Gen.Stmt

/-- The single-block run of the word: the volatile read is dropped and the
constant 1 is pushed — the source run is `simp`-closed (the pure-fragment
step semantics is definitionally transparent; `State.pop1` on `push1`
pairs with the List lemmas). The result holds for ANY entry state σ₀,
memory mem, and fuel budget. The exit state shares the `(State.pop1 σ₀).1`
projection on both sides, so the equality closes definitionally. -/
theorem run_to_const (spec : Tyu.IR.TargetSpec) (mem : Tyu.Step.ConcreteMem)
    (σ₀ : Tyu.Step.State) (fuel' : Nat) :
    Tyu.Src.Word.run [ { id := 0, ops := [Tyu.Src.Op.volLoad, Tyu.Src.Op.drop,
        Tyu.Src.Op.constInt 1, Tyu.Src.Op.ret] } ] spec 0 (fuel' + 1) mem σ₀ =
    (mem, some { stack := (Tyu.Step.State.pop1 σ₀).1.stack ++ [1],
                 locals := (Tyu.Step.State.pop1 σ₀).1.locals }) := by
  simp [Tyu.Src.Word.run, Tyu.Src.Block.runBlock, Tyu.Src.stepOp]
  simp [Tyu.Step.State.pop1, Tyu.Step.State.push1]

/-- The exit value of the reduced run: the pushed constant 1 (the
statement-side "exit top = 1" reading). -/
theorem run_exit_output (σ₀ : Tyu.Step.State) :
    Tyu.Src.outputAt { stack := (Tyu.Step.State.pop1 σ₀).1.stack ++ [1],
                       locals := (Tyu.Step.State.pop1 σ₀).1.locals } 0 = 1 := by
  simp [Tyu.Src.outputAt]

theorem obl_Uart7_read_tx_idle_subtype_range_1 :
    src_stmt_Uart7_read_tx_idle_subtype_range_1 := by
  unfold src_stmt_Uart7_read_tx_idle_subtype_range_1
  intro spec mem fuel σ₀ σf hr
  cases fuel with
  | zero =>
      simp [Tyu.Src.Word.run] at hr
  | succ fuel' =>
      have hrun : Tyu.Src.Word.run [ { id := 0, ops := [Tyu.Src.Op.volLoad, Tyu.Src.Op.drop,
            Tyu.Src.Op.constInt 1, Tyu.Src.Op.ret] } ] spec 0 (fuel' + 1) mem σ₀ =
          (mem, some { stack := (Tyu.Step.State.pop1 σ₀).1.stack ++ [1],
                       locals := (Tyu.Step.State.pop1 σ₀).1.locals }) :=
        run_to_const spec mem σ₀ fuel'
      rw [hrun] at hr
      injection hr with _ hσf
      injection hσf with hσ
      subst σf
      unfold Tyu.Src.inRange
      simp [run_exit_output σ₀]

end Tyu.Gen.Corpus.Uart7