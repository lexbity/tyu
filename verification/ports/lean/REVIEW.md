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
| T-CL (transitive) | the assumption-closure registry statement (§Q7 rule 2): on a well-closed verdict set, every closed obligation's entire assumption graph (walked through closed nodes) terminates on obligations that are closed or `runtime-check` terminals | `Tyu.Sound.AssumptionClosure.transitive_closure_sound` | proven, axiom-audited |
| T-CL (cycle) | a closed obligation reachable from itself — a cycle — contradicts well-closedness (cycles are E6419-malformed) | `Tyu.Sound.AssumptionClosure.cyclic_not_well_closed` | proven, axiom-audited |
| T-CL (open edge) | an assumption edge leaving a closed obligation to an open obligation contradicts well-closedness (the dependent resolves `assumption-unresolved`) | `Tyu.Sound.AssumptionClosure.open_edge_not_well_closed` | proven, axiom-audited |
| T-CL (base cases) | `runtime-check` obligations are terminals; an edge-less obligation is its own base case | `Tyu.Sound.AssumptionClosure.runtime_terminal`, `Tyu.Sound.AssumptionClosure.no_edges_terminal` | proven, axiom-audited |
| T-S (empty) | the source→IR transcription registry statement (§Q2, §Q16): a pure-fragment word's `Source-semantics(TargetSpec, MemModel)` equals the IR-semantics of its transcription | `Tyu.Sound.transcription` (↔ `Tyu.Src.transcription`), the forward/backward projections `Tyu.Sound.transcription_forward/backward`, and the offset form `Tyu.Sound.transcription_offset` | proven, axiom-audited |
| T-S (per-op) | per-op simulation: each fragment op's source step equals the concrete step of its transcription (the op-locality claim, case-by-case) | `Tyu.Src.transcription_op` | proven, axiom-audited |
| T-S (block) | the per-op lemmas composed over a block body (structural induction on the op list) | `Tyu.Src.transcription_block` | proven, axiom-audited |
| T-S (word) | the block lemmas composed over the word CFG walk (well-founded induction on the fuel budget; parameterized over `(TargetSpec, MemModel)`, §Q3) + the run-level iff | `Tyu.Src.transcription_run`, `Tyu.Src.transcription_run_iff` | proven, axiom-audited |

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
| `Tyu.Sound.AssumptionClosure.VerdictSet` (abstract verdict-set structure) | `Tyu/Sound.lean` | P8: the T-CL substrate — `closed`/`runtime`/`assumptions`, the closed-path reachability `closedPathTo`, `wellClosed` (acyclic ∧ edge-sound). Reviewed against §Q7 rule 2; the image walker (`tyu::closure`) executes it on concrete artifacts. |
| `Tyu.Stackmeta` parser + checker | `Tyu/Stackmeta.lean` | consumes `tyu.stackmeta/1` (schema-checked); unresolved calls skip a word — the gate FAILS on any skip (`ci/port.sh`) |
| `Tyu.Conformance.*` (abstract transfer oracle) | `Tyu/Conformance/` | mirrors `verifier::interp`/`interval`; zero-divergence pinned by 236 `tyu.vec/1` vectors |
| `Tyu.Conformance.Fragment` (fragment-corpus runner, `--level fragment`) | `Tyu/Conformance/Fragment.lean` | P9.3: consumes `tyu.fragvec/1` (program → trace) — parses the SAME canonical `--emit=ir` block text as the Rust side (`parseSrcBlocks`), runs `Tyu.Src.Word.run` (the source semantics), compares the committed observable traces. **The fragment's Lean↔Rust mechanical pin**: a mnemonic↔op or step drift on either side diverges here. This corpus caught the `swap` step bug (§4). |
| `Tyu.Gen.Stmt` (statement semantics + the invariant composition) | `Tyu/Gen/Stmt.lean` | P5: the rendered statements live here. The composition theorem `Tyu.Gen.Stmt.via_cycle_sound` is axiom-audited data among the §3 block below (proved by fuel induction). `Word.runFrom`/`runBlockSt` are structural statement-side copies of the port run semantics — the correspondence is **machine-checked**: `Word.runBlockOps_eq`, `Word.runBlockSt_eq`, and `Word.runFrom_is_runWord` (axiom-audited, §3) pin the copies to `Tyu.Step.Block.runBlock`/`Tyu.Step.runWord` budget for budget, and `inputAt_eq_src`/`outputAt_eq_src` pin the state projections to `Tyu.Src`'s. |
| `Tyu.Gen.Render` (statement renderer) | `Tyu/Gen/Render.lean` | P5: renders `Gen/<Module>.lean` from `tyu.obl/v2`; re-derives canonical statements + `statement_hash` in Lean (own SHA-256) — the renderer↔encoder drift lock, verified byte-exact against `crates/verifier/src/stmt.rs` over the corpus (`crates/tooling-tests/tests/gen_render_drift.rs`). The canonical dispatches on the formula op (`InRange` / `OffsetLE` / `PredicateHolds` — `stmt.rs::push_formula` byte-for-byte, incl. `{"name":"$top","op":"Var"}` opaque args). Omission classification (opaque `$top`, `call`-unmodeled words, dynamic MMIO, `stack-budget`) is part of the normative rendering rules (§Q4 item 4). **Contract statements (P8.2 scope):** `contract-post` renders `∀ σ₀ σf, run w … = some σf → predicateHolds wpred … [exit outputs]`; `contract-pre` renders the ∀-scheme over the argument tuple (a sound over-claim); both evaluate the predicate via `Tyu.Gen.Stmt.predicateHolds` (the predicate word run over the argument values). A named-predicate contract word contains the predicate `call` op — the statement-side step approximates `call` (representative sig), so such words stay `calls-unmodeled` (omitted honestly, never hashed to a wrong statement): the faithful statement-side mechanism for predicate calls (splicing the predicate's blocks inline at render time — the only calls in contract words) is a committed follow-on (`crates/tooling-tests/tests/renderer_scope.rs` pins the classification + the mmio/encoder hash lock). Inline (`unnamed`) clauses carry no artifact IR (`facts.predicates` records named words only) — the recording of inline clause IR is the second named follow-on. |
| `Tyu.Src` (fragment semantics + statement forms + transcription) + `Tyu.Src.transcription*` | `Tyu/Src.lean` | **P9 (T-S):** the pure-fragment *source-level* embedding (PLAN-VERIFY-3 §Q8): `Src.Op` (values, typed-stack ops, arithmetic/comparison/bool logic, memory over the oracles, the return-slot local-cell ops, CFG control), `Src.stepOp` (written directly — *not* the IR step), `Src.Block.runBlock`/`Src.Word.run` (mirrors of the IR runs), the source statement forms (`outInRange`, `offsetWithin`, `inInputRange`, `predHolds(path)`), the op-local **transcription** (`transcribeOp`/`transcribeBlock`, one IR op per source construct), and the per-op/block/word simulation theorems (T-S). The op-locality detector is the transcription being total: casts/traps/calls/address ops are EXCLUDED from the fragment (their lowering is not op-local — §Q2's shrink), and the renderer refuses source-surface statements for words containing them. Reviewed against `Tyu.Step`, `ir-op-semantics.md`, and §Q13 (memory oracles are parameters, never fixed values). |
| `Tyu.Gen.Sha256` (port SHA-256) | `Tyu/Gen/Sha256.lean` | FIPS 180-4, known-answer vectors pinned in `gen --selfcheck`; FR-14 (the only digest). |
| `Tyu.Gen.Golden` / `goldens/` (statement goldens) | `Tyu/Gen/Golden/*.lean`, `goldens/gen/*.gen.json`, `goldens/obl/*.obl.json` | P5: the committed generated statements, elaborated by the port gate; byte-stable; `ci/port.sh` rediffs them against the live `gen` output. |

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
Tyu.Gen.Stmt.via_cycle_sound
Tyu.Automation.via_cycles_sound
Tyu.Sound.AssumptionClosure.transitive_closure_sound
Tyu.Sound.AssumptionClosure.cyclic_not_well_closed
Tyu.Sound.AssumptionClosure.open_edge_not_well_closed
Tyu.Sound.AssumptionClosure.runtime_terminal
Tyu.Sound.AssumptionClosure.no_edges_terminal
Tyu.Sound.transcription
Tyu.Sound.transcription_forward
Tyu.Sound.transcription_backward
Tyu.Sound.transcription_offset
Tyu.Src.transcription_op
Tyu.Src.transcription_block
Tyu.Src.transcription_run
Tyu.Src.transcription_run_iff
Tyu.Gen.Stmt.Word.runBlockOps_eq
Tyu.Gen.Stmt.Word.runFrom_is_runWord
Tyu.Gen.Stmt.inputAt_eq_src
Tyu.Gen.Stmt.outputAt_eq_src
```

Permitted axiom set: `{propext, Quot.sound, Classical.choice}` — anything
else (`sorryAx`, `Lean.ofReduceBool`, a declared axiom) fails the gate.

> **Form deviation (documented, consistent with P4/P8):** the plan's per-slice
> gate `lake exe axiom_audit -- --theorem <name>` does not exist. The audit
> is `AxiomAudit.lean`'s `#print axioms` traversal over the entire §3 block,
> run by `ci/port.sh`: every REVIEW.md §3 theorem MUST appear in the audit
> output with the permitted set only, and an audit that silently drops a
> theorem fails the gate (fail-closed against audit erosion). There is no
> per-theorem exe invocation; adding a registry theorem means adding its
> `#print axioms` line to `AxiomAudit.lean` in the same commit.

## 3b. P5-composition soundness (axiom-audited)

The composition machinery adds no axioms: `Tyu.Gen.Stmt.via_cycle_sound`
(unfold `ViaCycle`; fuel induction with the `StepTo`/`StepOut`
guard-carrying routing) is proven and rechecked by the §3 axiom-audit block
below. The port-wide `sorry`-scan that `ci/guards.sh` G31 enforces covers
it: tyu/Gen carries no placeholders.

## 4. Empirical anchors

- `tyu.vec/1` — 236 vectors, four triples, zero divergence (`ci/port.sh`).
- `tyu.fragvec/1` — 17 fragment programs (program → trace), zero divergence
  (`conformance --level fragment`, `ci/port.sh`). **Bug found by this
  corpus:** the port's `swap` step (`Tyu.Step.stepOp` and `Tyu.Src.stepOp`)
  was the identity — `pushMany s2 [a, b]` instead of `[b, a]` — while the
  normative row says `( a b → b a )` and the Rust engine swaps; the
  `data-ops` vector (swap observable via `swap; sub`) diverged the corpus.
  Fixed in both step definitions; `transcription_op` still closes by `cases`
  per-op (the two surfaces changed identically), and the whole `tyu.vec/1` +
  stackmeta + fragment + axiom surfaces re-ran green. The corpus is the
  fragment's mechanical Lean↔Rust pin, exactly as `tyu.vec/1` is for the IR
  transfer.
- `tyu.stackmeta/1` — 52 corpus words, net exact + peak within declared,
  **zero unresolved-skip allowed** (`ci/port.sh` fails on any skip).
- Statement goldens — the Rust encoder's canonical statements are the
  byte-pinned interface (P1.3); the Gen renderer (P5) binds against them.
- `tyu.gen/1` + `Gen/*.lean` — 8 corpus modules rendered; **every rendered
  `statement_hash` is byte-equal to the Rust encoder's** (cross-implementation
  drift lock, pinned by `crates/tooling-tests/tests/gen_render_drift.rs`).
- **P5.2 worked-proof note:** the raw per-word scheme-instantiation proof for
  `LoopSub.bounded-count` (the plan's worked-proof gate) is a follow-on
  slice item: the rendered scheme + the `via_cycles` composition elaborate
  and the generic `via_cycle_sound` is proved, but the hand-proof of the
  concrete member-step preservation is deferred while the block evaluator's
  definitional transparency is finalized (no placeholder is shipped).
