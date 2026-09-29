# Tyu Lean 4 verification port (PLAN-VERIFY-3, Phase P3)

The first recognized port of the developer-proof pipeline. It consumes **generated data
only** — no Rust crate at build time (`verification/ports/*` read generated
artifacts; the generator edge is test-time and drift-locked).

## Layout

```
verification/ports/lean/
  lean-toolchain            toolchain pin (elan; falls back to a matching system lean)
  lakefile.toml             package: lean_lib "Tyu" + lean_exe "conformance"
  Tyu.lean                  library root — the generated data layer
  Tyu/IR/Op.lean            GENERATED — the op-form enum (42 rows)
  Tyu/IR/Semantics.lean     GENERATED — the semantics table + SEMANTICS_total
  Tyu/IR/Target.lean        GENERATED — the four recognized TargetSpec records
  Tyu/Mem.lean              GENERATED — MemModel interface + law-statement
                            placeholders (structure fields, never axioms)
  Main.lean                 the conformance executable
  Tyu/Conformance/          authored transfer engine over the generated layer:
    Interval.lean           the abstract interval lattice (mirrors verifier::interval)
    IntervalLaws.lean       in-build kernel-checked law pins
    Step.lean               op transfer (mirrors verifier::interp::State::step)
    Cfg.lean                worklist fixpoint + widening (verifier::interp::run_cfg)
    Parse.lean  VectorRun.lean  Json.lean  Runner.lean
                            the tyu.vec/1 reader + vector runner
```

The `Tyu/IR/*.lean` and `Tyu/Mem.lean` files are rendered by
`crates/verifier/src/export/lean.rs` and **must not be edited** — regenerate
with `TYU_EXPORT_PORTS=1 cargo test -p verifier --test export_drift` and
review the diff.

## What Phase P3 ships

- **P3.1 — generated data layer + completeness**: the op enum, the semantics
  table (mirroring `src/semantics/ops.json`, `tyu.ir-sem/1.0`), the target
  parameters, and the memory-model interfaces. `SEMANTICS_total` proves every
  op form has a row (kernel-checked `by decide`); a hand-edited generated
  file dropping a row fails the port build. All generated files are
  byte-drift-locked against the exporter.
- **P3.2 — conformance runner + port gate**: `conformance` reads the
  per-target `tyu.vec/1` corpora, reproduces every abstract transfer
  observation (row-level and CFG word-level), and requires **zero
  divergence**; a tampered expectation exits nonzero. `ci/port.sh` is the
  blocking port gate (toolchain resolution, `lake build`, conformance).

## Build / run

```
cd verification/ports/lean
lake build Tyu conformance
.lake/build/bin/conformance --corpus \
  ../../../crates/verifier/test-vectors/x86_64-unknown-none \
  ../../../crates/verifier/test-vectors/x86_64-unknown-linux-gnu \
  ../../../crates/verifier/test-vectors/armv7m-unknown-none \
  ../../../crates/verifier/test-vectors/riscv32-unknown-none
```

Or, from the repo root: `bash ci/port.sh` (blocking CI gate).

## Status vs the plan

| Item | Phase | State |
|---|---|---|
| Op enum / semantics table / targets / Mem interfaces (GEN) | P3.1 | done |
| `SEMANTICS_total` completeness | P3.1 | done (axiom-free, `by decide`) |
| Drift lock (`TYU_EXPORT_PORTS=1` + byte-compare) | P3.1 | done |
| Conformance runner over `tyu.vec/1` (236 vectors × 4 triples) | P3.2 | done — zero divergence |
| `ci/port.sh` port gate | P3.2 | done (conformance + axiom audit + stackmeta + fragment corpus) |
| Concrete `Tyu/Step.lean` semantics + `Terminates` | P4.1 | done |
| T-C: stack algebra (`Tyu/Sound.lean`, monoid + per-op + walk + net-zero loop) | P4.2 | done — axiom audit green (permitted set only) |
| T-C empirical hook (`--level stackmeta`, `tyu.stackmeta/1` goldens) | P4.2 | done — 52 words, zero divergence |
| Registry + trust documents rev 1 | P4.3 | done — `devdocs/plans/design-doc/formal-semantics-core.md`, `verification-trust.md`, `devdocs/plans/REVIEW.md` |
| T-CL: assumption closure (`Tyu/Sound.lean` §AssumptionClosure) | P8.1 | done — axiom audit green |
| **T-S: source-fragment embedding (`Tyu/Src.lean`) + transcription** | P9.1 | done — `Tyu.Sound.transcription` + per-op/block/word theorems, axiom audit green |
| Source-surface statements (`src_stmt_*`, `Tyu/Gen/Render.lean`) + harvest binding (`surface: "source"`, `relies: ["T-S"]`) | P9.1/P9.2 | done — rendered + harvested (Sum fixture) |
| Cross-surface vectors (`crates/verifier/tests/cross_surface.rs`, `src_interp`) + source-surface e2e | P9.2 | done — zero divergence; worked example green |
| **Fragment-vector corpus (`tyu.fragvec/1`, `conformance --level fragment`)** | P9.3 | done — 17 programs, zero divergence; the Lean↔Rust mechanical pin. **Caught and fixed the `swap` step bug** (Lean `swap` was the identity; the authoritative `( a b → b a )` row and Rust agree now) |
| `Word.runFrom` ≡ `Tyu.Step.runWord` correspondence | P5 follow-on | done — **`Tyu.Gen.Stmt.Word.runFrom_is_runWord`** (machine-checked, axiom-audited; promoted from review-recorded T-F2) |
| Gen renderer (`Tyu/Gen/`), harvest, axiom audit exe | P5/P7 | done |
| **Automation library (`Tyu/Automation/`): `stack_norm`/`arith`/`mem_frame`/`unfold_contracts` + `tyu_auto` dispatcher + `auto_cycle` via-cycles instantiation** | P10.1 | done — kernel-gated; closes the geometry class (`offsetWithin`) and the via-cycles compositions (`via_cycles_sound`, machine-checked); other statements fail honestly |
| **`automation_rate` exe** (P10.1 measurement, `tyu.automation-rate/1`, baseline-compared in `ci/port.sh`) | P10.1 | done — 11 corpus statements, 1 auto-closed (via-cycles); baseline `ci/automation-rate.json` |
| **`fill` exe** (P10.2 candidates: marker-headed `-- tyu:candidate obligation=<id>` files into `proofs/candidates/`) + `tyu proof fill` | P10.2 | done — never touches developer files |
| **Candidate attribution** (harvest `authoredOf`, `TYU_HARVEST_CANDIDATES` env; `tyu.candidates/1` evidence; report `candidates` field) + `--proven-no-candidates` deploy knob | P10.2 | done — `candidate_e2e` green (positive + negative); candidate verdicts stay `trust: proof` |
| **P10.2 exit bar: loop-free auto-discharge ≥ 90% (NFR-6)** | P10.2 | **NOT MET — reported in `ci/port.sh` (vacuity-corrected): the honest per-kind number prints `FAIL` every run, non-blocking per §Q10 ("informational, not a build gate"); the baseline no-drift bar stays blocking (see §P10 re-scope below)** |
| **`tyu.vm/1` producer (P11.1)** — the verdicts-v2 → manifest-summary converter: per-module `<Module>.vm.json` emitted beside the report; real (computed) `id_hash`/`statement_hash`, auto-packed root manifest under requiring policies | P11.1 | done — `vm_manifest` test asserts the hashes equal the canonical encoder's; `verify_manifest_from_json` now consumes derived summaries, not hand-authored JSON |
| **`RUNTIME_ABI_VERSION` 1→2 (plan §13 row)** | P11.1 | **adjudicated, NOT bumped** — recorded in `crates/lmod/src/abi_hash.rs`: `MODINFO_VER` already 4 (the v4 change already realized "modinfo feeds abi_hash → all modules recompile"), and `verify_manifest` is modinfo metadata, not a `__lang_*` contract change (abi-contract §4.4.3); a bump would force a full fleet recompile against an unchanged ABI |
| **`tyu proof fill <input.mod>` (standalone extraction)** | P10.2/P11 | done — `--target` + real `langc --emit=obligations` graph extraction replaces the dead stub; `tyu proof fill entry.mod` works without a prior build |
| **Deploy image-level pairing (FR-8, §Q7 rule 3)** — `tyu deploy` walks the import graph under a requiring policy; proven deploys reject unproven/unmodeled CALLEES by name (E6510); the root caller is vetted via its packed manifest | P11.3 | done — `deploy_verify_policy` covers single-module + the two-module compositional walk; the loader hook staying `Off` by default is documented (deploy is the enforcement point) |
| **Hosted reference policy (P11.2)** — `HostedLoaderPlatform::reference()` = `RequireNoOpen` (E6502 enforcement surface); `.with_verify_policy()` for other policies; `.new()` keeps the loader `Off` default for legacy loads | P11.2 | done — `reference_policy_is_require_no_open` hosted test; the loader-core policy matrix (load.rs) covers the enforcement |
| verify_manifest record + loader policy (E6500/01/02) + deploy pairing (E6510) | P11 | done — see `crates/lmod/src/verify_manifest.rs`, `loader-core`, `tyu deploy --verify-policy` |
| **Single interval implementation (`Tyu/Abs.lean`)** — the P14 consolidation: the lattice, the abstract state/transfer (`stepOp`/`absRun`), the `∇` widening, and `Contains`; `Conformance/Interval.lean` + `Conformance/Step.lean` are re-export shims (`export`/`abbrev`) | P14.1 | done — corpus re-ran byte-exact over the absorbed definitions (consolidation discharged, REVIEW.md §2) |
| **T-A (abstraction soundness)** — the transfer table is sound: `add`/`sub`/`mul`/`castNarrow`/`join`/`widen`/`triCmp`/`triAnd`/`triOr`/`triNot`/`triFromBoolIv` `*_sound` + the four named wrap-boundary examples + the word-domain `±1` boundary facts (machine-checked `by decide`) | P14.1 | done — `Tyu.Sound.TA.*`, axiom audit green (permitted set only) |
| **T-B (discharge soundness)** — `evalInRange = defTrue` ⟹ every concretization satisfies the target bounds (the `rederive` bridge) | P14.2 | done — `Tyu.Sound.TB.discharge_sound`, axiom audit green |
| **`rederive` exe** — `--corpus` (the differential pin: re-run `tyu.vec/1` program corpora) + `--obl` (artifact → `tyu.verdicts/v2`, `trust: proof, method: rederive, proof.kind: rederive`, T-B statement, statement-bound) | P14.2 | done — `--selfcheck` + `--corpus` + `--obl` green |
| **The re-derivation differential** — `verifier::gen` generated programs run by BOTH engines (Rust in-tree discharge + the port's `Tyu.Abs` re-run) must agree exactly (tri = trust class, interval = value); committed corpus `test-vectors/rederive` (256, byte-stable) + the ≥10^5 env-forced run | P14.2 | done — 100k programs in ~7 s, zero divergence; `ci/differential.sh` gates it (NFR-3 budget asserted) |
| **Automation-only `proven` build** — a module discharged entirely by `rederive` (no developer proofs) builds under `--verify-policy=proven`; without it the interval `checked` discharges are forced open (checks retained, §Q12) | P14.2 | done — `proven_automation_only` e2e green |

## The source surface, precisely

A developer's word renders a source-surface statement (`src_stmt_…`) iff its
`--emit=ir` text parses entirely as fragment ops (`Tyu.Gen.Render.parseSrcOp`):
`const` / `dup` / `drop` / `swap`, `add` / `sub` / `mul` / `cmp_*` /
`and` / `or` / `not`, `load` / `store` / `vol_load` / `vol_store`,
`local_get` / `local_set`, `br` / `br_if` / `ret`. Casts (`1 as Percent`),
calls, and address/effect ops are NOT fragment ops — such words stay on the
IR surface (the §Q2 shrink, stated rather than discovered). An obligation has
ONE theorem name (`obl_…`) per build, so an IR-surface and a source-surface
proof of the same obligation cannot coexist; migrating surfaces means
renaming the theorem.

## §P10 re-scope record (recorded 2026-09-27, PLAN-VERIFY-3 P10.2)

Conscious, documented divergence from the P10.2 exit bar — recorded rather
than silently weakened:

- **Bar:** "≥ 90% loop-free auto-discharge measured (asserted by the python
  gate in CI)" (NFR-6), plus `auto_cycle` invariant templates
  (range-carrying / accumulator / monotone), `--fill-budget`, honest failure
  records, and "the §11.1 example's cycle lemma fills from a range template".
- **What shipped:** the full candidate/attribution pipeline, `--fill-budget`
  plumbing, per-statement wall-clock budgets (`tactic-timeout` /
  `elaboration-error` taxonomy), the loopfree×kind rate split, the
  template-invariant surface + a range-template cycle-lemma demonstration —
  and the honest measurement: **`automation_rate` on the committed corpus
  closes 1/11 statements (the via-cycles composition); loop-free
  `subtype-range` is 0/9.**
- **Why the bar is not met:** the automation closes the classes the
  embeddings make decidable without general fuel reasoning (`offsetWithin`
  geometry, the machine-checked via-cycles compositions). Direct loop-free
  `outInRange` statements demand the concrete-run discharge engine
  (`Word.runFrom` is fuel-indexed; per-block fuel induction + concrete-block
  peeling + routing-target binding). Repeated attempts in this slice could
  not land that engine kernel-clean within budget; it is the **named
  follow-on workstream** (a verified symbolic executor over the renderer's
  block literals).
- **The gate is honest:** `ci/port.sh` runs the vacuity-corrected ≥0.9
  python assertion over the loop-free rows; absent kinds pass vacuously,
  non-empty kinds are REPORTED against the bar with the measured number.
  Posture correction (2026-09-27, second record): the absolute bar is
  **reported, not blocking** — §Q10 is normative here ("Quality is measured,
  published, not gated … the bar is economic honesty, not a pass/fail
  gate"; the P10.2 process gate is "informational, not a build gate"), and a
  permanently-red blocking port gate would price every later slice against
  an economics number instead of soundness. The blocking parts of the P10
  gate are the FUNCTIONAL contract (fill markers, baseline no-drift, the
  candidate/proof-fill e2e tests); the printed `FAIL` line is the published
  honest rate until the discharge-engine workstream lands.
- The baseline `ci/automation-rate.json` is the drift lock; the README's
  "11 statements, 1 auto-closed" is the measured truth.
