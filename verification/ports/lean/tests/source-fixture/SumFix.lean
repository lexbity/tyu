import Gen.Sum

/-
The P9.2 source-surface worked example: the `Sum::answer::subtype-range::0`
obligation proved at the SOURCE surface (`src_stmt_Sum_answer_subtype_range_0`,
the pure-fragment statement rendered by the `gen` renderer over `Tyu/Src.lean`).

The developer never touches tyu source for the invariant or the proof: the
word is `const_i64 42; ret` (a fragment word — every op is in
`Tyu.Src.Op`), and the statement is `∀ σ₀, run terminates → 0 ≤ outputAt σf 0 ≤ 100`.
The proof evaluates the fragment run (a `simp`-closed run computation —
`Tyu.Src.const_ret_exit`), observes the exit value is the pushed constant
(`Tyu.Src.const_ret_output`), and closes the range by arithmetic. It is a
SOURCE-level certificate: the harvest binds it with `surface: "source"` and
`relies: ["T-S"]` (§Q2 — the claim discharges the IR obligation in
composition with the transcription theorem).

No `sorry`/`Admitted`/`native_decide`: the axiom audit gates it.
-/

namespace Tyu.Gen.Corpus.Sum

theorem obl_Sum_answer_subtype_range_0 : src_stmt_Sum_answer_subtype_range_0 := by
  unfold src_stmt_Sum_answer_subtype_range_0
  intro spec mem fuel σ₀ σf hr
  cases fuel with
  | zero =>
      simp [Tyu.Src.Word.run] at hr
  | succ fuel' =>
      have hrun : Tyu.Src.Word.run [ { id := 0, ops := [Tyu.Src.Op.constInt 42, Tyu.Src.Op.ret] } ]
            spec 0 (fuel' + 1) mem σ₀ =
            (mem, some { stack := σ₀.stack ++ [42], locals := σ₀.locals }) :=
        Tyu.Src.const_ret_exit 42 spec mem σ₀ fuel'
      rw [hrun] at hr
      injection hr with _ hst
      injection hst with hσf
      subst σf
      unfold Tyu.Src.inRange
      rw [Tyu.Src.const_ret_output 42 spec mem σ₀ fuel']
      omega

end Tyu.Gen.Corpus.Sum