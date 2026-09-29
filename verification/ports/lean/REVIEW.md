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
| T-A (add, P14.1) | interval add over-approximates the concrete sum of any concretizations (the wrap→⊤ rule proven sound: an overflowed bound computation is `⊤`, which contains everything) | `Tyu.Sound.TA.add_sound` | proven, axiom-audited |
| T-A (sub, P14.1) | interval sub over-approximates the concrete difference | `Tyu.Sound.TA.sub_sound` | proven, axiom-audited |
| T-A (mul, P14.1) | the four-corner tableau over-approximates the concrete product | `Tyu.Sound.TA.mul_sound` | proven, axiom-audited |
| T-A (cast, P14.1) | the narrowing-cast meet still contains in-range concretizations (out-of-range concretizations trap on the concrete cast) | `Tyu.Sound.TA.cast_narrow_sound` | proven, axiom-audited |
| T-A (join, P14.1) | a concretization of either join operand is a concretization of the hull | `Tyu.Sound.TA.join_sound` | proven, axiom-audited |
| T-A (widen, P14.1) | the back-edge widening `∇` only grows — a discharge over the widened value is a discharge over every pre-widening value | `Tyu.Sound.TA.widen_sound` | proven, axiom-audited |
| T-A (comparison, P14.1) | the abstract comparison's bool interval contains the concrete comparison result of any concretization (defTrue/defFalse/`⊤` each sound) | `Tyu.Sound.TA.tri_cmp_sound` | proven, axiom-audited |
| T-A (triFromBoolIv, P14.1) | the abstract bool of a bool-typed interval contains its 0/1 concretizations | `Tyu.Sound.TA.tri_from_bool_iv_sound` | proven, axiom-audited |
| T-A (triAnd, P14.1) | the concrete `&&` result is within the abstract `triAnd` interval | `Tyu.Sound.TA.tri_and_sound` | proven, axiom-audited |
| T-A (triOr, P14.1) | the concrete `||` result is within the abstract `triOr` interval | `Tyu.Sound.TA.tri_or_sound` | proven, axiom-audited |
| T-A (triNot, P14.1) | the concrete `not` result is within the abstract `triNot` interval | `Tyu.Sound.TA.tri_not_sound` | proven, axiom-audited |
| §Q14 FIFO (round-trip, P15) | the abstract-atomic channel semantics: send-then-recv on a make-local channel observes exactly the sent payload, and the round trip leaves the FIFO empty (atomicity: the op pair touches only its own channel, tasks, and clock are untouched) | `Tyu.Services.send_then_recv_value`, `Tyu.Services.send_then_recv_empty`, `Tyu.Services.roundtrip_other_unchanged`, `Tyu.Services.chanOps_clock_monotone`, `Tyu.Services.timeAdvance_monotone` | proven, axiom-audited |
| §Q14 FIFO (order, P15) | two sends then two recvs return the values in order — the semantics is a FIFO, never LIFO or reordered | `Tyu.Services.fifo_order` | proven, axiom-audited |
| §Q14 trace (P15.2) | the service-op trace is the word's sequential reasoning surface: `[make 0, send 0 v, recv 0]` leaves `v` on the stack (the FIFO law composed over the trace), and the bounded in-range transport + the registry `fifo_roundtrip` statement make the renderer's `traceInRange` form provable | `Tyu.Services.trace_send_recv_output`, `Tyu.Services.trace_send_recv_in_range`, `Tyu.Services.fifo_roundtrip` | proven, axiom-audited |
| T-B (discharge, P14.2) | the discharge bridge: `eval_in_range` answering `defTrue` for an interval means every concretization satisfies the target bounds — the lemma the `rederive` method composes (a `⊤`/`⊥` interval can never reach `defTrue`) | `Tyu.Sound.TB.discharge_sound` | proven, axiom-audited |
| T-CL (transitive) | the assumption-closure registry statement (§Q7 rule 2): on a well-closed verdict set, every closed obligation's entire assumption graph (walked through closed nodes) terminates on obligations that are closed or `runtime-check` terminals | `Tyu.Sound.AssumptionClosure.transitive_closure_sound` | proven, axiom-audited |
| T-CL (cycle) | a closed obligation reachable from itself — a cycle — contradicts well-closedness (cycles are E6419-malformed) | `Tyu.Sound.AssumptionClosure.cyclic_not_well_closed` | proven, axiom-audited |
| T-CL (open edge) | an assumption edge leaving a closed obligation to an open obligation contradicts well-closedness (the dependent resolves `assumption-unresolved`) | `Tyu.Sound.AssumptionClosure.open_edge_not_well_closed` | proven, axiom-audited |
| T-CL (base cases) | `runtime-check` obligations are terminals; an edge-less obligation is its own base case | `Tyu.Sound.AssumptionClosure.runtime_terminal`, `Tyu.Sound.AssumptionClosure.no_edges_terminal` | proven, axiom-audited |
| T-S (empty) | the source→IR transcription registry statement (§Q2, §Q16): a pure-fragment word's `Source-semantics(TargetSpec, MemModel)` equals the IR-semantics of its transcription | `Tyu.Sound.transcription` (↔ `Tyu.Src.transcription`), the forward/backward projections `Tyu.Sound.transcription_forward/backward`, and the offset form `Tyu.Sound.transcription_offset` | proven, axiom-audited |
| T-S (per-op) | per-op simulation: each fragment op's source step equals the concrete step of its transcription (the op-locality claim, case-by-case) | `Tyu.Src.transcription_op` | proven, axiom-audited |
| T-S (block) | the per-op lemmas composed over a block body (structural induction on the op list) | `Tyu.Src.transcription_block` | proven, axiom-audited |
| T-S (word) | the block lemmas composed over the word CFG walk (well-founded induction on the fuel budget; parameterized over `(TargetSpec, MemModel)`, §Q3) + the run-level iff | `Tyu.Src.transcription_run`, `Tyu.Src.transcription_run_iff` | proven, axiom-audited |
| T-D (store-load) | the bundle-instance memory laws (§6.8/§Q16, P12.2): a point store within a modeled bundle's RAM window is read back by a load of that address (a real abstract store, not `⊤`) | `Tyu.Sound.TD.store_load` | proven, axiom-audited |
| T-D (frame) | a store to one point address leaves the load behavior of every other point address unchanged (the frame property) | `Tyu.Sound.TD.frame_law` | proven, axiom-audited |
| T-D (aperture bound) | an MMIO read answers exactly the §Q13 width-bounded nondeterministic domain — a value within the register's width, never wider | `Tyu.Sound.TD.aperture_width_bound`, `Tyu.Sound.TD.aperture_width_bound_full` | proven, axiom-audited |
| T-D (bundle geometry) | each modeled bundle instance's RAM window equals its `model/model.toml [memory] ram` declaration (pinned mechanically by the Rust suite against `--level bundles`) and the window's points are in-RAM per the instance | `Tyu.Sound.TD.x86_64_geometry`, `Tyu.Sound.TD.armv7m_geometry`, `Tyu.Sound.TD.riscv32_geometry` (+ the instance in-window lemmas `*_inram`, axiom-audited, §3) | proven, axiom-audited |
| T-D (mirror equivalence) | the conformance runner's operational `MemModel.bundle` and the theorem-surface `BundleMem` agree on observable reads (aperture via the single `wordDomain`; loads via the recorded-cells coercion) — the two mirrors cannot drift (the boundary-byte defect class) | `Tyu.Sound.TD.bundle_aperture_agrees`, `Tyu.Sound.TD.bundle_load_agrees` | proven, axiom-audited |
| T-D (rp2350 geometry, P13.2) | the rp2350 board pack's bundle instance window equals its `model/model.toml [memory] ram` declaration (the first 64 KiB of SRAM) and the window's points are in-RAM | `Tyu.Sound.TD.rp2350_geometry`, `Tyu.Sound.TD.rp2350_inram` | proven, axiom-audited |
| T-D (width/mask, P13.2) | a refined register read answers exactly the refinement's modeled band `[0, mask]` — within the register's width, never wider than the §Q13 width-bounded default, **instanced at the device** (`uartfr_band_domain_at_device`: `[0, 0xF9]` — the band IS consumed by the registry, P3 finding), and the refined read is proven **within the width domain** for any band (`band_within_width_domain` — `width` mathematically consumed, P3s finding 6d) | `Tyu.Sound.TD.uartfr_band_domain`, `Tyu.Sound.TD.uartfr_band_domain_at_device`, `Tyu.Sound.TD.band_within_width_domain` | proven, axiom-audited |
| T-D (access-mode, ro, P13.2) | a refined READ-ONLY register answers exactly the refinement's band — `refinedOf` consults the access-mode (`AccessMode`): `ro` closes the read to the datasheet stable-value set; place matching is the SAME trailing-token semantics as the binding (`refined_read_at_any_matching_place`: a read at ANY matching place is the band — the P12 `bundle_load_agrees` agreement pattern on the refinement surface) | `Tyu.Sound.TD.ro_read_answers_band`, `Tyu.Sound.TD.refined_read_at_any_matching_place` | proven, axiom-audited |
| T-D (access-mode, write-capable, P13.2) | every WRITE-CAPABLE refined register answers the §Q13 width-bounded nondeterministic domain (the model records no register state; the w1c/w1s bit-clears are structural, stated in the TCB boundary) | `Tyu.Sound.TD.write_capable_read_width_bounded` | proven, axiom-audited |
| T-D (access-mode, writes, P13.2) | reads of a refined device are memory-independent — RAM stores and the (unmodeled) register write state only change memory, which no read consults (abstract); the concrete write channel is isolated: a `vol_store` then a `vol_load` answers exactly the oracle's read | `Tyu.Sound.TD.read_independent_of_memory`, `Tyu.Sound.TD.store_then_read_is_oracle` | proven, axiom-audited |
| T-D (volatility/ordering, P13.2) | two consecutive MMIO reads are NOT merged — each `vol_load` records its own read in the run's ACCESS TRACE (`Tyu.Step.runBlockTrace`, execution order); the trace of `[vol_load, vol_load]` has TWO entries, and the trace run is proven equal to `Block.runBlock` (the trace is a formalization of the run's reads, never a parallel semantics) — the §Q13 no-elide/merge/reorder guarantee as the trace (the final stack alone cannot distinguish merged reads; the trace can) | `Tyu.Sound.TD.runBlockTrace_eq_runBlock`, `Tyu.Sound.TD.two_reads_trace` | proven, axiom-audited |

Helper lemmas (not registry entries, reviewed definitions): `id_wf`,
`compose_wf` (as `Bound.compose_wf`), `branch_max_wf`, `compose_high_ge_*`,
`opBound_wf`, `foldBound_net`, `foldBound_wf`.

## 2. Reviewed definitions (T-F2 surface — the kernel-valid, intent-reviewed set)

| Definition | File | Review basis |
|---|---|---|
| `Tyu.Step.ConcreteMem` (cells + load/mmio oracles) | `Tyu/Step.lean` | §Q13: nondeterminism is a parameter; reviewed against `verifier::mem::MemModel` |
| `Tyu.Step.stepOp` (42-form concrete transfer) | `Tyu/Step.lean` | per-op reviewed against `crates/verifier/src/semantics/mod.rs` rows; depth claims *proved* (§3 per-op form), value claims pinned by `tyu.vec/1` where the op is observable |
| `Tyu.Step.runOk` / `Outcome.trap` | `Tyu/Step.lean` | trap = retained check firing (`trap_if_false`) or stack underflow; `some` = terminating non-trapping run |
| `Tyu.Step.runBlockTrace` (the MMIO-read access trace) + `runBlockTrace_eq_runBlock` | `Tyu/Step.lean` | **P13.2:** the trace-collecting sibling of `Block.runBlock` — one read VALUE per `vol_load`/`vol_load_field` in execution order; identical routing/memory/end, proven equal to `Block.runBlock` (`Tyu.Sound.TD.runBlockTrace_eq_runBlock`, §3), so the trace is a formalization of the run's reads, not a parallel semantics. This is the "volatility ordering as a TRACE property" (the final stack cannot distinguish merged reads; the trace can) — P2 finding discharged. |
| `Tyu.Sound.Bound` + monoid | `Tyu/Sound.lean` | the §13.2 monoid, mirrored from `stack-bound-analysis.md` |
| `Tyu.Sound.AssumptionClosure.VerdictSet` (abstract verdict-set structure) | `Tyu/Sound.lean` | P8: the T-CL substrate — `closed`/`runtime`/`assumptions`, the closed-path reachability `closedPathTo`, `wellClosed` (acyclic ∧ edge-sound). Reviewed against §Q7 rule 2; the image walker (`tyu::closure`) executes it on concrete artifacts. |
| `Tyu.Stackmeta` parser + checker | `Tyu/Stackmeta.lean` | consumes `tyu.stackmeta/1` (schema-checked); unresolved calls skip a word — the gate FAILS on any skip (`ci/port.sh`) |
| `Tyu.Conformance.*` (abstract transfer oracle) | `Tyu/Conformance/` | mirrors `verifier::interp`/`interval`; zero-divergence pinned by 236 `tyu.vec/1` vectors |
| `Tyu.Conformance.Fragment` (fragment-corpus runner, `--level fragment`) | `Tyu/Conformance/Fragment.lean` | P9.3: consumes `tyu.fragvec/1` (program → trace) — parses the SAME canonical `--emit=ir` block text as the Rust side (`parseSrcBlocks`), runs `Tyu.Src.Word.run` (the source semantics), compares the committed observable traces. **The fragment's Lean↔Rust mechanical pin**: a mnemonic↔op or step drift on either side diverges here. This corpus caught the `swap` step bug (§4). |
| `Tyu.Gen.Stmt` (statement semantics + the invariant composition) | `Tyu/Gen/Stmt.lean` | P5: the rendered statements live here. The composition theorem `Tyu.Gen.Stmt.via_cycle_sound` is axiom-audited data among the §3 block below (proved by fuel induction). `Word.runFrom`/`runBlockSt` are structural statement-side copies of the port run semantics — the correspondence is **machine-checked**: `Word.runBlockOps_eq`, `Word.runBlockSt_eq`, and `Word.runFrom_is_runWord` (axiom-audited, §3) pin the copies to `Tyu.Step.Block.runBlock`/`Tyu.Step.runWord` budget for budget, and `inputAt_eq_src`/`outputAt_eq_src` pin the state projections to `Tyu.Src`'s. |
| `Tyu.Gen.Render` (statement renderer) | `Tyu/Gen/Render.lean` | P5: renders `Gen/<Module>.lean` from `tyu.obl/v2`; re-derives canonical statements + `statement_hash` in Lean (own SHA-256) — the renderer↔encoder drift lock, verified byte-exact against `crates/verifier/src/stmt.rs` over the corpus (`crates/tooling-tests/tests/gen_render_drift.rs`). The canonical dispatches on the formula op (`InRange` / `OffsetLE` / `PredicateHolds` — `stmt.rs::push_formula` byte-for-byte, incl. `{"name":"$top","op":"Var"}` opaque args). Omission classification (opaque `$top`, `call`-unmodeled words, dynamic MMIO, `stack-budget`) is part of the normative rendering rules (§Q4 item 4). **Contract statements (P8.2 scope):** `contract-post` renders `∀ σ₀ σf, run w … = some σf → predicateHolds wpred … [exit outputs]`; `contract-pre` renders the ∀-scheme over the argument tuple (a sound over-claim); both evaluate the predicate via `Tyu.Gen.Stmt.predicateHolds` (the predicate word run over the argument values). A named-predicate contract word contains the predicate `call` op — the statement-side step approximates `call` (representative sig), so such words stay `calls-unmodeled` (omitted honestly, never hashed to a wrong statement): the faithful statement-side mechanism for predicate calls (splicing the predicate's blocks inline at render time — the only calls in contract words) is a committed follow-on (`crates/tooling-tests/tests/renderer_scope.rs` pins the classification + the mmio/encoder hash lock). Inline (`unnamed`) clauses carry no artifact IR (`facts.predicates` records named words only) — the recording of inline clause IR is the second named follow-on. |
| `Tyu.Src` (fragment semantics + statement forms + transcription) + `Tyu.Src.transcription*` | `Tyu/Src.lean` | **P9 (T-S):** the pure-fragment *source-level* embedding (PLAN-VERIFY-3 §Q8): `Src.Op` (values, typed-stack ops, arithmetic/comparison/bool logic, memory over the oracles, the return-slot local-cell ops, CFG control), `Src.stepOp` (written directly — *not* the IR step), `Src.Block.runBlock`/`Src.Word.run` (mirrors of the IR runs), the source statement forms (`outInRange`, `offsetWithin`, `inInputRange`, `predHolds(path)`), the op-local **transcription** (`transcribeOp`/`transcribeBlock`, one IR op per source construct), and the per-op/block/word simulation theorems (T-S). The op-locality detector is the transcription being total: casts/traps/calls/address ops are EXCLUDED from the fragment (their lowering is not op-local — §Q2's shrink), and the renderer refuses source-surface statements for words containing them. Reviewed against `Tyu.Step`, `ir-op-semantics.md`, and §Q13 (memory oracles are parameters, never fixed values). |
| `Tyu.Bundles.BundleMem` (functional bundle-memory model) + the `x86_64`/`armv7m`/`riscv32` instances | `Tyu/Bundles/` | **P12 (T-D):** the abstract memory state of a modeled bundle — `cell : Int → IntervalVal` plus the RAM window; loads honor only point stores inside the window, stores replace the cell (newest wins), MMIO reads answer the width-bounded nondeterministic domain. This is the denotational shape the Rust `verifier::mem::ApertureMem` (list-of-cells) implements; the two are pinned to agree by the committed `tyu.vec/1` bundle corpora (Rust `bundle_instance_conformance` suite + the port's `conformance` exe = zero divergence) and by the `--level bundles` geometry report against `model/model.toml`. Reviewed against `verifier::mem` and §Q13. |
| `Tyu.Bundles.rp2350` (the rp2350 board instance) + `uartFrBand` (0xF9) / `uartFrMode` (ro) + `BundleMem.apertureReadRefined` / `refinedOf` + `AccessMode` (the refinement surface) | `Tyu/Bundles/Bundle.lean`, `Tyu/Bundles/Rp2350.lean` | **P13.2 (T-D completion + refinements):** the rp2350 modeled bundle — SRAM window `[0x20000000, 0x2000FFFF]` (pinned mechanically against `model/model.toml` by the Rust suite), the one `[refinements]` device (`UARTFR` → `rp2350.uart-fr`, datasheet band `mask = 0xF9` — the SIX modeled flag bits of the RP2350's UARTFR, DS2 §12.1, NOT the full-PL011 0xFF8 — and access-mode `ro`, both transcribed in the manifest and pinned against the Lean instance by `--level bands`), `AccessMode` (the closed source-level access set `{ro, wo, rw, w1c, w1s, rc}`), `apertureReadRefined` (the §Q13 width-bounded read narrowed to the refinement's band), and `refinedOf` (mode-gated: ro ⇒ the band, write-capable ⇒ the §Q13 default, writes inert). Registry entries §3: the geometry/inram laws + the width/mask (instanced at the device), access-mode (ro/write-capable/writes), and volatility trace laws. |
| `Tyu.Conformance.MemModel.bundle` (operational bundle model in the abstract transfer) | `Tyu/Conformance/Step.lean` | **P12.2:** the conformance runner's operational mirror of the bundle instance — `store` mutates the recorded cells, `load` consults them, `apertureRead` answers the width domain (the single `Tyu.Conformance.wordDomain`, shared with `Bundles.BundleMem.apertureRead`). Behavior-preserving for the shared corpora (all 236 pre-existing vectors re-pass unchanged); bundle corpora exercise the new flavor. **Do not re-implement the bundle rules here**: the denotational twin is `Tyu.Bundles.BundleMem`, and the two mirrors' agreement is a **registry theorem** (`Tyu.Sound.TD.bundle_aperture_agrees` / `bundle_load_agrees`, §3) — a change to one mirror without the other fails those lemmas. |
| `Tyu.Abs` (the single abstract interval implementation + `stepOp` + `widenOld` + `Contains`) | `Tyu/Abs.lean` | **P14 (T-A/T-B):** the port's ONE interval layer (absorbing `Conformance/Interval.lean` + the `Conformance/Step.lean` transfer; the conformance files are re-export shims — P4-audit consolidation, 2026-09-28). Mirrors `verifier::interval`/`verifier::interp::State::step`: the lattice, the wrap→⊤ arithmetic, the `∇` back-edge widening, the abstract bool/comparison transfer, and the `Contains` concretization predicate the soundness theorems are stated over. `stepOp` matches the generated `OpForm` wildcard-free (a semantics row without an abstract arm is a compile error). The corpus re-ran byte-exact; soundness is the §3 T-A/T-B family. |
| `Tyu.Gen.Sha256` (port SHA-256) | `Tyu/Gen/Sha256.lean` | FIPS 180-4, known-answer vectors pinned in `gen --selfcheck`; FR-14 (the only digest). |
| `Tyu.Gen.Golden` / `goldens/` (statement goldens) | `Tyu/Gen/Golden/*.lean`, `goldens/gen/*.gen.json`, `goldens/obl/*.obl.json` | P5: the committed generated statements, elaborated by the port gate; byte-stable; `ci/port.sh` rediffs them against the live `gen` output. |

**P14 consolidation (discharged, 2026-09-28):** the abstract interval layer
now lives ONCE in `Tyu/Abs.lean` (namespace `Tyu.Abs` — the lattice, the
abstract state/transfer `stepOp`/`absRun`, the `widenOld` `∇` operator, and
the `Contains` concretization predicate). `Tyu/Conformance/Interval.lean`
and `Tyu/Conformance/Step.lean` are re-export shims (`export`/`abbrev`) —
the runner, the stackmeta path, the bundle instances, and the fragment
corpus all execute the single implementation. The T-A/T-B registry
statements (`Tyu.Sound.TA`/`Tyu.Sound.TB`) prove its soundness; the
conformance corpus re-ran byte-exact against the absorbed definitions.

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
Tyu.Sound.TD.store_load
Tyu.Sound.TD.frame_law
Tyu.Sound.TD.aperture_width_bound
Tyu.Sound.TD.aperture_width_bound_full
Tyu.Sound.TD.x86_64_geometry
Tyu.Sound.TD.x86_64_inram
Tyu.Sound.TD.armv7m_geometry
Tyu.Sound.TD.armv7m_inram
Tyu.Sound.TD.riscv32_geometry
Tyu.Sound.TD.riscv32_inram
Tyu.Sound.TD.rp2350_geometry
Tyu.Sound.TD.rp2350_inram
Tyu.Sound.TD.uartfr_band_domain
Tyu.Sound.TD.uartfr_band_domain_at_device
Tyu.Sound.TD.band_within_width_domain
Tyu.Sound.TD.ro_read_answers_band
Tyu.Sound.TD.refined_read_at_any_matching_place
Tyu.Sound.TD.write_capable_read_width_bounded
Tyu.Sound.TD.read_independent_of_memory
Tyu.Sound.TD.store_then_read_is_oracle
Tyu.Sound.TD.runBlockTrace_eq_runBlock
Tyu.Sound.TD.two_reads_trace
Tyu.Sound.TD.bundle_aperture_agrees
Tyu.Sound.TD.bundle_load_agrees
Tyu.Sound.TA.add_sound
Tyu.Sound.TA.sub_sound
Tyu.Sound.TA.mul_sound
Tyu.Sound.TA.cast_narrow_sound
Tyu.Sound.TA.join_sound
Tyu.Sound.TA.widen_sound
Tyu.Sound.TA.tri_cmp_sound
Tyu.Sound.TA.tri_from_bool_iv_sound
Tyu.Sound.TA.tri_and_sound
Tyu.Sound.TA.tri_or_sound
Tyu.Sound.TA.tri_not_sound
Tyu.Sound.TB.discharge_sound
Tyu.Services.send_then_recv_value
Tyu.Services.send_then_recv_empty
Tyu.Services.fifo_order
Tyu.Services.roundtrip_other_unchanged
Tyu.Services.chanOps_clock_monotone
Tyu.Services.timeAdvance_monotone
Tyu.Services.trace_send_recv_output
Tyu.Services.trace_send_recv_in_range
Tyu.Services.fifo_roundtrip
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
- **P13 (device refinements, §Q13):** the refined worked example
  (`verification/ports/lean/tests/refined-fixture/` — the `Uart7` artifact
  + `Uart7Fix` proof) is a **SOURCE-surface certificate whose refinement is
  MATHEMATICALLY load-bearing** (P3s finding 5): the word returns the raw
  UARTFR read, and the generated statement is the BAND-RESTRICTED
  `Tyu.Src.outInRangeRefined blocks 0 0 0 255 0 249` — "runs whose reads
  answer within the refinement's datasheet band `[0, 0xF9]` return a
  subtype-valid byte"; the theorem applies the band hypothesis and closes by
  arithmetic — NOT provable over the §Q13 universal oracle (an out-of-band
  read falsifies it). Harvested with `surface: "source"`,
  `relies: ["T-S"]` and the statement hash carrying
  `refinement: "rp2350.uart-fr"` (§Q13/P13.1). The word is fragment-
  parseable BECAUSE the fixture omits the `addr_of`/`mmio_place` address
  materialization langc's lowering inserts ahead of a real MMIO `vol_load`
  — those ops are NOT in `Tyu.Src.Op`, so a *full lowering's* MMIO word is
  refused at the source surface (`non-fragment-ops`; the fragment boundary
  of `Tyu/Src.lean`, §Q2). The example demonstrates the source-surface
  mechanism on the fragment word; the address-materialization boundary
  stays the honest refusal for full lowerings.
- **P13.2 refined-read VALUE evidence (P3s finding 4):** the rp2350 bundle
  corpus (`platforms/rp2350/evidence/vectors.json`) now includes vectors
  that READ `uart.UARTFR` with scripted reads (`tyu.vec/1` `"reads"`,
  mirroring `ApertureMem.script_read` and exercised identically by the Lean
  runner, which threads the op place + the scripted table): in-band reads
  discharge, an OUT-OF-BAND scripted read falsifies (the band is meaningful,
  not cosmetic), and an unscripted read answers the §Q13 width-bounded
  default. The Rust↔Lean agreement over refined reads therefore covers a
  READ VALUE, not just the statement hash (64 bundle vectors, zero
  divergence).
- **P13.1 render refusal ("mismatch ⇒ render refuses"):** the renderer
  OMITS a MODELED bundle's MMIO-word statements when no refinement context
  document was given (`refined-read-unbound`), never silently re-binding
  them to the unrefined default — a modeled artifact ALWAYS waits for the
  bundle's `tyu.refinements/1` context (`TYU_GEN_REFINEMENTS`; tyu passes it
  via `refinement_context`). Unmodeled bundles render the honest §Q13
  nondeterministic-read default either way.
- The gen renderer's refinement context (`Tyu.Gen.Render.RefinementDecl` /
  `parseRefinements` / `refinementOf` / `refinementRefuses`) is part of the
  rendering-rules surface (§Q4 item 4) — reviewed with the renderer (T-F2
  discipline), byte-drift-locked by
  `crates/tooling-tests/tests/refined_proof_e2e.rs`.
