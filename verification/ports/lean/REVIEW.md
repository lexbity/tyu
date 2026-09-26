# Lean port review artifact — registry ↔ theorem map (rev 1)

> PLAN-VERIFY-3 P4.3: the mapping from the statement registry
> (`devdocs/plans/design-doc/formal-semantics-core.md`, §3) to this port's
> theorem names, plus the review status of every hand-written definition.
> The **Axiom-audited theorems** block below is machine-consumed:
> `ci/port.sh` extracts it and requires every listed theorem to appear in
> the axiom audit (`AxiomAudit.lean`) with the permitted axiom set only.
> Adding a registry entry means adding its theorem to that block in the
> same commit — the gate fails otherwise.

## 1. Registry entries → theorems

| Registry id | Claim (see `formal-semantics-core.md` §3) | Theorem(s) | Status |
|---|---|---|---|
| T-C (sequence) | terminating run: exit = entry + net (exact), peak ≤ entry + high | `Tyu.Sound.stack_algebra_sequence` | proven, axiom-audited |
| T-C (walk) | same, over any terminating CFG walk of a word | `Tyu.Sound.stack_algebra_walk` | proven, axiom-audited |
| T-C (iswalk) | walk-form depth laws over `IsWalk` (stronger: holds for any id sequence) | `Tyu.Sound.stack_algebra_iswalk` | proven, axiom-audited |
| T-C (per-op) | 42-case per-row depth lemmas: `stepOk` moves depth exactly by `net`, peak within `entry + max 0 net`, underflow traps | `Tyu.Sound.stepOk_length`, `Tyu.Sound.stepOk_peak` | proven, axiom-audited |
| T-C (run) | run-level transport: `runOk` length/peak laws | `Tyu.Sound.runOk_length`, `Tyu.Sound.runOkPeak_bound` | proven, axiom-audited |
| T-C (monoid) | §13.2 stack algebra: associativity, identity, wf preservation | `Tyu.Sound.Bound.compose_assoc`, `compose_id_left`, `compose_id_right`, `Bound.compose_wf` | proven, axiom-audited |
| T-C (loops) | net-zero cycle repetition: net unchanged, depth unchanged | `Tyu.Sound.repeat_net_zero`, `Tyu.Sound.repeat_net_exact` | proven, axiom-audited |
| T-C (fold) | depth/peak fold coherence: `seq_net_exact`, `seq_peak_envelope`, `walk_net`, `walk_peak` | same names in `Tyu.Sound` | proven, axiom-audited |

Helper lemmas (not registry entries, reviewed definitions): `id_wf`,
`compose_wf` (as `Bound.compose_wf`), `branch_max_wf`, `compose_high_ge_*`,
`opBound_wf`, `foldBound_net`, `foldBound_wf`.

## 2. Reviewed definitions (T-F2 surface — the kernel-valid, intent-reviewed set)

| Definition | File | Review basis |
|---|---|---|
| `Tyu.Step.ConcreteMem` (cells + load/mmio oracles) | `Tyu/Step.lean` | §Q13: nondeterminism is a parameter; reviewed against `verifier::mem::MemModel` |
| `Tyu.Step.stepOp` (42-form concrete transfer) | `Tyu/Step.lean` | per-op reviewed against `crates/verifier/src/semantics/mod.rs` rows; depth claims *proved* (§3 per-op form), value claims pinned by `tyu.vec/1` where the op is observable |
| `Tyu.Step.runOk` / `Outcome.trap` | `Tyu/Step.lean` | trap = retained check firing (`trap_if_false`) or stack underflow; `some` = terminating non-trapping run |
| `Tyu.Sound.Bound` + monoid | `Tyu/Sound.lean` | the §13.2 monoid, mirrored from `stack-bound-analysis.md` |
| `Tyu.Stackmeta` parser + checker | `Tyu/Stackmeta.lean` | consumes `tyu.stackmeta/1` (schema-checked); unresolved calls skip a word — the gate FAILS on any skip (`ci/port.sh`) |
| `Tyu.Conformance.*` (abstract transfer oracle) | `Tyu/Conformance/` | mirrors `verifier::interp`/`interval`; zero-divergence pinned by 236 `tyu.vec/1` vectors |

**Known consolidation point (P14, locked in the plan):** the abstract
interval layer currently lives in `Tyu/Conformance/Interval.lean` +
`Tyu/Conformance/Step.lean`; when P14 lands `Tyu/Abs.lean`, it MUST absorb
`Conformance/Interval.lean` and re-point the conformance runner — the port
must not carry two interval implementations.

## 3. Axiom-audited theorems (machine-consumed block — keep in sync with AxiomAudit.lean)

```text
Tyu.Sound.stack_algebra_sequence
Tyu.Sound.stack_algebra_iswalk
Tyu.Sound.stack_algebra_walk
Tyu.Sound.runOk_length
Tyu.Sound.runOkPeak_bound
Tyu.Sound.walk_net
Tyu.Sound.walk_peak
Tyu.Sound.repeat_net_zero
Tyu.Sound.stepOk_length
Tyu.Sound.stepOk_peak
Tyu.Sound.Bound.compose_assoc
Tyu.Sound.Bound.compose_id_left
Tyu.Sound.Bound.compose_id_right
Tyu.Sound.Bound.compose_wf
Tyu.Sound.seq_net_exact
Tyu.Sound.seq_peak_envelope
```

Permitted axiom set: `{propext, Quot.sound, Classical.choice}` — anything
else (`sorryAx`, `Lean.ofReduceBool`, a declared axiom) fails the gate.

## 4. Empirical anchors

- `tyu.vec/1` — 236 vectors, four triples, zero divergence (`ci/port.sh`).
- `tyu.stackmeta/1` — 52 corpus words, net exact + peak within declared,
  **zero unresolved-skip allowed** (`ci/port.sh` fails on any skip).
- Statement goldens — the Rust encoder's canonical statements are the
  byte-pinned interface (P1.3); the Gen renderer (P5) binds against them.
