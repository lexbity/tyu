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
