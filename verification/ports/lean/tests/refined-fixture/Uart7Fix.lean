import Gen.Uart7

/-
The P13.2 refined worked example, proven at the SOURCE surface, with the
refinement MATHEMATICALLY load-bearing (P3s finding 5): `Uart7::read-tx-idle`
returns the raw UARTFR read as an `RxByte` (range 0..255). The generated
statement is the BAND-RESTRICTED source claim

    src_stmt_Uart7_read_tx_idle_subtype_range_1 :
      Tyu.Src.outInRangeRefined blocks 0 0 0 255 0 249

        ∀ mem, (0 ≤ mem.mmioRead ∧ mem.mmioRead ≤ 249) →
          (run terminates) → inRange (outputAt σf 0) 0 255

i.e.: every terminating run whose UARTFR read answers within the refinement's
datasheet band `[0, 0xF9]` (the six modeled flag bits, DS2 §12.1) returns a
subtype-valid byte. The claim is provable BECAUSE of the refinement: without
the band restriction it would have to hold over the §Q13 universal oracle
(an oracle answering `read = 1000` would falsify it) — the §Q13 escape the
plan demands ("an ensures that depends on a read value proven because of
the refinement"). The theorem unfolds the fragment run (the read is the
output), applies the band hypothesis, and closes by arithmetic — the
out-of-band oracle runs are simply outside the claim's domain.

The harvest certifies it with `surface: "source"`, `relies: ["T-S"]`, and
the statement hash carrying `refinement: "rp2350.uart-fr"`.

Note on the fragment boundary (recorded, `REVIEW.md` §4): the fixture's
word IR is fragment-parseable BECAUSE it omits the `addr_of`/`mmio_place`
address materialization langc's lowering inserts ahead of a real MMIO
`vol_load` (those ops are NOT in `Tyu.Src.Op` — real-MIO words are refused
at the source surface `non-fragment-ops`). The example demonstrates the
source-surface mechanism on the fragment word; the address-materialization
boundary stays the honest refusal for full lowerings.

No proof placeholders: the axiom audit gates the theorem's axiom closure. -/

namespace Tyu.Gen.Corpus.Uart7

open Tyu.Gen.Stmt

/-- The single-block run of the word: `vol_load` pops the entry address and
pushes the oracle read — the run's exit stack carries the read as its top.
The source run is `simp`-closed; the result holds for ANY entry state σ₀,
memory mem, and fuel budget. -/
theorem run_to_read (spec : Tyu.IR.TargetSpec) (mem : Tyu.Step.ConcreteMem)
    (σ₀ : Tyu.Step.State) (fuel' : Nat) :
    Tyu.Src.Word.run [ { id := 0, ops := [Tyu.Src.Op.volLoad, Tyu.Src.Op.ret] } ]
      spec 0 (fuel' + 1) mem σ₀ =
    (mem, some { stack := (Tyu.Step.State.pop1 σ₀).1.stack ++ [mem.mmioRead],
                 locals := (Tyu.Step.State.pop1 σ₀).1.locals }) := by
  simp [Tyu.Src.Word.run, Tyu.Src.Block.runBlock, Tyu.Src.stepOp]
  simp [Tyu.Step.State.pop1, Tyu.Step.State.push1]

/-- The exit value of the reduced run: the oracle read (the word's output IS
the refined register's read). -/
theorem run_exit_output (σ₀ : Tyu.Step.State) (v : Tyu.Step.Value) :
    Tyu.Src.outputAt { stack := (Tyu.Step.State.pop1 σ₀).1.stack ++ [v],
                       locals := (Tyu.Step.State.pop1 σ₀).1.locals } 0 = v := by
  simp [Tyu.Src.outputAt]

theorem obl_Uart7_read_tx_idle_subtype_range_1 :
    src_stmt_Uart7_read_tx_idle_subtype_range_1 := by
  unfold src_stmt_Uart7_read_tx_idle_subtype_range_1
  intro spec mem fuel σ₀ σf hband hr
  cases fuel with
  | zero =>
      simp [Tyu.Src.Word.run] at hr
  | succ fuel' =>
      have hrun : Tyu.Src.Word.run [ { id := 0, ops := [Tyu.Src.Op.volLoad, Tyu.Src.Op.ret] } ]
            spec 0 (fuel' + 1) mem σ₀ =
          (mem, some { stack := (Tyu.Step.State.pop1 σ₀).1.stack ++ [mem.mmioRead],
                       locals := (Tyu.Step.State.pop1 σ₀).1.locals }) :=
        run_to_read spec mem σ₀ fuel'
      rw [hrun] at hr
      injection hr with _ hσf
      injection hσf with hσ
      subst σf
      unfold Tyu.Src.inRange
      simp [run_exit_output σ₀ mem.mmioRead]
      -- The refinement is load-bearing: the OUT-OF-BAND oracle domain is
      -- outside the claim; in-band reads bound the output by the band.
      -- (omega abstracts over a named `let` — `mem.mmioRead`'s function
      -- application is opaque to it — so the band constraints are re-stated
      -- on the abstracted value.)
      let a : Int := mem.mmioRead
      change 0 ≤ a ∧ a ≤ 255
      have hb0a : 0 ≤ a := by simpa [a] using hband.1
      have hb1a : a ≤ 249 := by simpa [a] using hband.2
      constructor <;> omega

end Tyu.Gen.Corpus.Uart7