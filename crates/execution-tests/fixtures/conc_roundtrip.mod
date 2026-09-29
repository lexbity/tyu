# PLAN-VERIFY-3 P15.2 — the concurrency proof template (iso payload handoff).
#
# `roundtrip-pct` makes a channel, sends the constant payload 42 into its
# FIFO, receives it back, and narrows the received value to the `Percent`
# subtype. The channel is make-local, so the FIFO model decides the received
# value: 42. The subtype-range obligation (`out.0 ∈ [0, 100]`) is proven by a
# theorem of the abstract-atomic services statement
# (`Tyu.Services.traceInRange [make 0, send 0 42, recv 0] 0 100`) — the
# §Q14 FIFO round-trip law (Tyu.Servoices.fifo_roundtrip).
#
# The word's OPS are all modeled channel services and the payload is a
# compile-time constant, so the static channel-trace lowering renders the
# service statement only under a bundle declaring
# `[model] concurrency = "abstract-atomic"` (the linux-x86_64-hosted bundle,
# P15.1); under any other concurrency declaration the same obligation fails
# closed open with the `service-unmodeled` witness (§Q14).
#
# `main` exits 0 iff the received value equals the sent payload — the runtime
# anchor the hosted QEMU/native leg asserts (FIFO/atomicity on the wire).
module Conc;
import platform/channel;
subtype Percent = i64 range 0..100;

: roundtrip-pct ( -- Percent )
  platform.channel.make dup
  42 platform.channel.send
  platform.channel.recv
  as Percent ;

: main ( -- i64 )
  roundtrip-pct as i64 42 ==
  not [ 1 ] [ 0 ] if ;
export { main };
end;