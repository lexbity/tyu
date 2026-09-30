# Changelog

All notable changes to tyu are recorded here.

Format: Keep a Changelog 1.1.0. Versions are `MAJOR.MINOR.0` (the git SHA
identifies fixes within a minor, per the release policy in `RELEASES.md`);
one `##` section per release; unreleased work accumulates under
`[Unreleased]` and is promoted when the release is cut.

## [Unreleased]

## [0.1.0] - 2026-09-30

First release of the tyu toolchain: an evidence-first, fail-closed compiler
and dynamic module loader for x86_64/ARM/RISC-V, with a Lean 4 verification
port, harnessed verification pipeline, and a newly-live CI + release
engineering surface.

Highlights of the current tree:

- Compiler pipeline: frontend -> typed IR -> three codegen backends with a
  QEMU execution matrix (hosted and bare-metal x86_64, ARM Cortex-M, RISC-V).
- Dynamic `.lmod` loading with signing and encryption, aperture binding, and
  a re-derivation differential against the Lean port (>= 1e5 programs).
- The statement band rule (PLAN-VERIFY-3 §Q4 item 3) made CI-enforceable:
  dated nightly pin (`nightly-2026-09-29`), clippy/rustfmt clean on the
  pin, SHA-pinned Actions, actionlint/shellcheck gates, and the
  `tyu.doctor`-ready health-check surface.
- Release engineering: tracked workflows triggering on every push, signed
  source tarballs, `ci/cut-release.sh` (the release runbook as code).

This is the first cut; the changelog discipline (one section per minor,
fixes recorded under the current minor) starts here.