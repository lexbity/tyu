# Tyu programming language

Tyu is a concatenative, stack-based systems language for embedded and simulation targets. This repository contains the compiler, loader, module tooling, runtime assembly, sysroot sources, and test suites.

## Core idea

A small **concatenative, stack-based** systems language for embedded + simulation:
- no GC
- explicit allocation via **regions**
- **fixed arrays** are core (`T'N`)
- higher-level collections/algorithms live in the **stdlib**
- Ada/SPARK-ish safety tools: **subtypes** + **contracts**
- strong MMIO/representation support

## Repository map

- `crates/`: compiler, loader, codegen, tooling (incl. `verifier`, `tyu`,
  `langc`, `loader-core`, `lmod*`), and tests (incl. `execution-tests`,
  `tooling-tests`)
- `verification/ports/lean/`: the reference proof port (Lean 4) — generated
  data layer, semantics + registry theorems, statement renderer, harvest,
  automation library, conformance/rederive exes
- `runtime/`: per-target assembly runtime and linker scripts
- `sysroot/`: Tyu standard library sources (incl. the hosted bundle's model
  identity carrier)
- `platforms/<triple>/`: platform packs — `platform.toml` (schema 3, incl.
  `[model]` semantics), model artifacts, `evidence/` vector corpora
- `test-goldens/`: checked-in expected artifacts (obligations, verdicts,
  statements, stackmeta, assumptions, modinfo, reports)
- `fuzz/`: cargo-fuzz decode targets (`obl_v2_decode`, `verdicts_v2_decode`,
  `cert_index_decode`, …) with seeded corpora
- `ci/guards.sh`, `ci/port.sh`, `ci/differential.sh`, `ci/acceptance.sh`:
  repository gates, the port gate, the ≥10^5 differential, the §11 acceptance
  matrix
- `ci/verify-corpus/` + `ci/verify-allowlist.txt`: the verification corpus and
  its open-obligation allowlist (guards.sh G20 — every reported open is
  justified, and every justification still occurs)
- `CONTRIBUTING.md`: contribution and test policy
- `devdocs/`: design plans (incl. the verification plan family), the v3 book,
  the technical manual, and the research docs

## Design documents (verification subsystem)

The static-verification plan family lives in `devdocs/plans/`:

- `devdocs/plans/developer-proof-pipeline.md` — PLAN-VERIFY-3 (current): the
  developer-proof pipeline — canonical statements (`tyu.stmt/1.0`), dual
  proof surfaces, model semantics, the harvest path, trust classes, the
  certification package. Phase slices P1–P15 implemented; P16 (anchor,
  blame surface, acceptances) closes it.
- `devdocs/plans/done/static-verification.md` — PLAN-VERIFY-1 (implemented):
  obligations, verdicts, policy, slices P1–P8.
- `devdocs/tyu-research/formal-verification.md` — PLAN-VERIFY-2 (partially
  superseded per PLAN-VERIFY-3 §Q16): formal semantics, certifier-neutral
  ports, the certification package's origin.
- `devdocs/plans/design-doc/ir-op-semantics.md` — the normative IR op
  semantics + VC projections (the *only* normative meaning of IR ops for
  proof purposes).
- `devdocs/plans/design-doc/formal-semantics-core.md` — the semantics core +
  statement registry (owner of `tyu.stmt`).
- `devdocs/plans/design-doc/verification-trust.md` — trust classes, certifier
  registry, manifest + package records.
- `devdocs/plans/design-doc/verification-obligations.md` — the `.obl.json` /
  verdicts / report field semantics.
- `devdocs/plans/design-doc/` also holds the pre-existing companions
  (abi-contract, effect-context-model, stack-bound-analysis,
  module-format-and-loading, runtime-diagnostic-protocol-v1).

## Build framework

- cross-compiler targeting platforms
- hosted management tools

## Prerequisites

- Rust stable, as pinned by `rust-toolchain.toml`
- `fasm`, `as`, and `ld`
- `qemu-system-x86_64`, `qemu-system-arm`, and `qemu-system-riscv32`
- Optional cross toolchains: `gcc-arm-none-eabi`, `gcc-riscv64-unknown-elf`
- Optional proof toolchain (the `ci/port.sh` / `ci/differential.sh` tier):
  `lean` + `lake` at the pin recorded in `verification/ports/lean/lean-toolchain`
- `TYU_BIN_DIR` for execution-test runs that need built host binaries

## Quick start

See [`SETUP.md`](SETUP.md) for the full setup path and first-run checks.

The commands below are taken from repo CI and have not been re-run in this write-up.

```bash
cargo build --release -p langc -p tyu -p lmod-pack -p lmod-encrypt -p lmod-sign  # unverified
cargo test --workspace --release  # unverified
```

Before opening a PR, run:

```bash
bash ci-lint.sh  # unverified
bash ci/guards.sh  # unverified
```
