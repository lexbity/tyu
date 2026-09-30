# Tyu programming language

[![CI](https://github.com/lexbity/tyu/actions/workflows/ci.yml/badge.svg?branch=main&event=push)](https://github.com/lexbity/tyu/actions/workflows/ci.yml)
[![Integration](https://github.com/lexbity/tyu/actions/workflows/integration.yml/badge.svg?branch=main&event=push)](https://github.com/lexbity/tyu/actions/workflows/integration.yml)
[![Install verify](https://github.com/lexbity/tyu/actions/workflows/install-verify.yml/badge.svg)](https://github.com/lexbity/tyu/actions/workflows/install-verify.yml)
[![Release](https://github.com/lexbity/tyu/actions/workflows/release.yml/badge.svg)](https://github.com/lexbity/tyu/actions/workflows/release.yml)

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
- `docs/book/`: the canonical book (14 chapters + appendices, 82 verified
  lab fixtures) — tracked and CI-exercised
- `devdocs/`: design plans (incl. the verification plan family), the
  technical manual, and the research docs (gitignored scratchpad)

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

The toolchain is used in three tiers (`hosted` / `metal` / `proof`); the
exact per-tier commands are in [`SETUP.md`](SETUP.md) and are the same
commands CI executes. Essentials:

- Rust at the **dated nightly pin** in `rust-toolchain.toml` (rustup
  provisions it; `rust-src` is included for the RISC-V loader build)
- `fasm`, `binutils` (provides `ld`, `nm`)
- `qemu-system-x`, `qemu-system-arm`, `qemu-system-misc` (the
  `qemu-system-*` binaries)
- Optional cross toolchains: `gcc-arm-none-eabi`, `gcc-riscv64-unknown-elf`
- Optional proof tier (the `ci/port.sh` / `ci/differential.sh` gates):
  elan + Lean at the pin in `verification/ports/lean/lean-toolchain`
- `TYU_BIN_DIR` for execution-test runs that need built host binaries

## Quick start

The blessed path is the **source installer** (no prebuilt binaries). Full,
CI-executed setup steps are in [`SETUP.md`](SETUP.md):

```bash
git clone https://github.com/lexbity/tyu
cd tyu
bash ci/install.sh                 # newest v* tag (else main); installs into ~/.tyu
export PATH="$HOME/.tyu/bin:$PATH"
# first build + run = the committed smoke fixture (the exact contract CI proves)
tyu build ci/smoke/hello.mod --out-dir=/tmp/smoke-out
/tmp/smoke-out/image.elf; echo $?         # must be 0 (green)
```

From a working checkout, the same commands CI runs before a merge:

```bash
cargo build --release -p langc -p tyu -p lmod-pack -p lmod-encrypt -p lmod-sign
cargo test --workspace --release
bash ci-lint.sh
bash ci/guards.sh
```

## Status

[![CI](https://github.com/lexbity/tyu/actions/workflows/ci.yml/badge.svg)](https://github.com/lexbity/tyu/actions/workflows/ci.yml)
[![Integration](https://github.com/lexbity/tyu/actions/workflows/integration.yml/badge.svg)](https://github.com/lexbity/tyu/actions/workflows/integration.yml)

Releases: [`RELEASES.md`](RELEASES.md) (policy) ·
[GitHub releases](https://github.com/lexbity/tyu/releases)
