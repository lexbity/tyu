import Tyu.Sound
import Tyu.Gen.Stmt
import Tyu.Automation.Cycle

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
#print axioms Tyu.Automation.via_cycles_sound
#print axioms Tyu.Sound.AssumptionClosure.transitive_closure_sound
#print axioms Tyu.Sound.AssumptionClosure.cyclic_not_well_closed
#print axioms Tyu.Sound.AssumptionClosure.open_edge_not_well_closed
#print axioms Tyu.Sound.AssumptionClosure.runtime_terminal
#print axioms Tyu.Sound.AssumptionClosure.no_edges_terminal
#print axioms Tyu.Sound.transcription
#print axioms Tyu.Sound.transcription_forward
#print axioms Tyu.Sound.transcription_backward
#print axioms Tyu.Sound.transcription_offset
#print axioms Tyu.Src.transcription_op
#print axioms Tyu.Src.transcription_block
#print axioms Tyu.Src.transcription_run
#print axioms Tyu.Src.transcription_run_iff
#print axioms Tyu.Sound.TD.store_load
#print axioms Tyu.Sound.TD.frame_law
#print axioms Tyu.Sound.TD.aperture_width_bound
#print axioms Tyu.Sound.TD.aperture_width_bound_full
#print axioms Tyu.Sound.TD.x86_64_geometry
#print axioms Tyu.Sound.TD.x86_64_inram
#print axioms Tyu.Sound.TD.armv7m_geometry
#print axioms Tyu.Sound.TD.armv7m_inram
#print axioms Tyu.Sound.TD.riscv32_geometry
#print axioms Tyu.Sound.TD.riscv32_inram
#print axioms Tyu.Sound.TD.rp2350_geometry
#print axioms Tyu.Sound.TD.rp2350_inram
#print axioms Tyu.Sound.TD.uartfr_band_domain
#print axioms Tyu.Sound.TD.uartfr_band_domain_at_device
#print axioms Tyu.Sound.TD.band_within_width_domain
#print axioms Tyu.Sound.TD.ro_read_answers_band
#print axioms Tyu.Sound.TD.refined_read_at_any_matching_place
#print axioms Tyu.Sound.TD.write_capable_read_width_bounded
#print axioms Tyu.Sound.TD.read_independent_of_memory
#print axioms Tyu.Sound.TD.store_then_read_is_oracle
#print axioms Tyu.Sound.TD.runBlockTrace_eq_runBlock
#print axioms Tyu.Sound.TD.two_reads_trace
#print axioms Tyu.Sound.TD.bundle_aperture_agrees
#print axioms Tyu.Sound.TD.bundle_load_agrees
#print axioms Tyu.Sound.TA.add_sound
#print axioms Tyu.Sound.TA.sub_sound
#print axioms Tyu.Sound.TA.mul_sound
#print axioms Tyu.Sound.TA.cast_narrow_sound
#print axioms Tyu.Sound.TA.join_sound
#print axioms Tyu.Sound.TA.widen_sound
#print axioms Tyu.Sound.TA.tri_cmp_sound
#print axioms Tyu.Sound.TA.tri_from_bool_iv_sound
#print axioms Tyu.Sound.TA.tri_and_sound
#print axioms Tyu.Sound.TA.tri_or_sound
#print axioms Tyu.Sound.TA.tri_not_sound
#print axioms Tyu.Sound.TB.discharge_sound
#print axioms Tyu.Gen.Stmt.Word.runBlockOps_eq
#print axioms Tyu.Gen.Stmt.Word.runFrom_is_runWord
#print axioms Tyu.Gen.Stmt.inputAt_eq_src
#print axioms Tyu.Gen.Stmt.outputAt_eq_src
