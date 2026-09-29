import Gen.Conc
import Tyu.Services

/-! The concurrency template worked proof (PLAN-VERIFY-3 P15.2, §Q14).

The module `Conc` (`conc_roundtrip.mod`) round-trips the constant payload 42
through a make-local channel and narrows the received value to `Percent`
([0, 100]). The renderer lowers the word's IR to the static channel trace
`[make 0, send 0 42, recv 0]` and emits the abstract-atomic services
statement `Tyu.Services.traceInRange [.make 0, .send 0 42, .recv 0] 0 100` —
"for ANY initial instance the trace leaves an output value in [0, 100]".

This theorem proves that statement by composing the §Q14 FIFO round-trip law
(`Tyu.Services.trace_send_recv_output`: the trace output IS the sent payload)
with the subtype bound. The payload hand-off is atomic: the channel is
make-local, so no other task's send/recv can interleave into it (the
`chanSend_tasks_clock_unchanged` / `roundtrip_other_unchanged` atomicity laws
of the model). -/

namespace Tyu.Gen.Corpus.Conc

/-- `Conc::roundtrip-pct::subtype-range::1` — the FIFO round-trip claim: the
received value is the sent payload (42) and lies in `Percent` [0, 100]. -/
theorem obl_Conc_roundtrip_pct_subtype_range_1
    : stmt_Conc_roundtrip_pct_subtype_range_1 := by
  unfold stmt_Conc_roundtrip_pct_subtype_range_1
  intro inst
  refine ⟨42, ?_⟩
  constructor
  · exact Tyu.Services.trace_send_recv_output 42 inst
  · decide

end Tyu.Gen.Corpus.Conc