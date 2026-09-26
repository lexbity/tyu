import Tyu.Sound
import Tyu.Gen.Stmt

/-! The P4.2 axiom-audit surface: every registry theorem must depend only
on the permitted axioms {propext, Quot.sound, Classical.choice}. Unproven
placeholder axioms and `Lean.ofReduceBool` are never permitted (§Q11 item
2); the audit runner
(`lake env lean AxiomAudit.lean`) prints the `#print axioms` sets and the
port gate greps for the forbidden constants.

The audited set is OWNED by `REVIEW.md` §3 (the machine-consumed registry
block): `ci/port.sh` extracts that block and requires every listed theorem
to appear in this audit. Keep the two lists in the same commit. -/

#print axioms Tyu.Sound.stack_algebra_sequence
#print axioms Tyu.Sound.stack_algebra_iswalk
#print axioms Tyu.Sound.stack_algebra_walk
#print axioms Tyu.Sound.runOk_length
#print axioms Tyu.Sound.runOkPeak_bound
#print axioms Tyu.Sound.walk_net
#print axioms Tyu.Sound.walk_peak
#print axioms Tyu.Sound.repeat_net_zero
#print axioms Tyu.Sound.stepOk_length
#print axioms Tyu.Sound.stepOk_peak
#print axioms Tyu.Sound.Bound.compose_assoc
#print axioms Tyu.Sound.Bound.compose_id_left
#print axioms Tyu.Sound.Bound.compose_id_right
#print axioms Tyu.Sound.Bound.compose_wf
#print axioms Tyu.Sound.seq_net_exact
#print axioms Tyu.Sound.seq_peak_envelope
#print axioms Tyu.Gen.Stmt.via_cycle_sound
