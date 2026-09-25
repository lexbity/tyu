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

- `crates/`: compiler, loader, codegen, tooling, and tests
- `runtime/`: per-target assembly runtime and linker scripts
- `sysroot/`: Tyu standard library sources
- `test-goldens/`: checked-in expected artifacts
- `ci-lint.sh` and `ci/guards.sh`: repository gates
- `ci/verify-corpus/` + `ci/verify-allowlist.txt`: the verification corpus and
  its open-obligation allowlist (guards.sh G20 — every reported open is
  justified, and every justification still occurs)
- `DOCS.md`: tracked documentation map and governance note
- `SETUP.md`: tracked setup and local development guide
- `TROUBLESHOOTING.md`: common failures and recovery paths
- `GLOSSARY.md`: shared terminology
- `LIMITATIONS.md`: honest gaps and cleanup items
- `CLEANUP.md`: cleanup status and maintainer follow-ups
- `CONTRIBUTING.md`: contribution and test policy

## Design documents (verification subsystem)

The static verification plan and its companion suite live in
`devdocs/plans/`:

- `devdocs/plans/static-verification.md` — the static-verification plan
  (PLAN-VERIFY-1): obligations, verdicts, policy, slices.
- `devdocs/plans/design-doc/ir-op-semantics.md` — the normative IR op
  semantics + VC projections (the *only* normative meaning of IR ops for
  proof purposes).
- `devdocs/plans/design-doc/verification-obligations.md` — owner of the
  `.obl.json` / verdicts / report field semantics.
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
