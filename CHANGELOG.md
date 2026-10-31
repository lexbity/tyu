# Changelog

All notable changes to tyu are recorded here.

Format: Keep a Changelog 1.1.0. Versions are `MAJOR.MINOR.0` (the git SHA
identifies fixes within a minor, per the release policy in `RELEASES.md`);
one `##` section per release; unreleased work accumulates under
`[Unreleased]` and is promoted when the release is cut.

## [Unreleased]

### Fixed

- **Platform runtime kind is declared, not inferred**: every platform pack
  manifest carries `[platform] kind = "hosted" | "metal"` — the single
  source of truth for which execution flow a runtime belongs to. The
  builder, linter, and reporting surfaces read the declaration; the
  file-layout inference (`is_hosted_pack`) is gone.
- **Hosted platform pack builds and runs** (`tyu build` / `tyu run
  --platform=linux-x86_64-hosted`): the builder now honors the pack's
  declared `[metal] startup` unit instead of hardcoding a `runtime` stem
  (the hosted pack could never assemble). The hosted runtime exports its
  `__lang_ds_base`/`__lang_ds_limit` contract symbols, so the pack also
  lints green.
- **`--mode=dynamic` is gated on a metal runtime, before any assembly**:
  dynamic images are metal-firmware images (the on-device loader and
  `.modpack` only exist in the metal flow), so a hosted/wasm
  `--mode=dynamic` now fails with the intentional gate diagnostic naming
  the declared kind instead of a confusing missing-`dynamic_entry.asm`
  error. The duplicated per-target error sites
  (`device_loader_rust_target`/`device_loader_profile`) are retired.
- **ARM ISR fixture runs in the QEMU pipeline** (`isr_lock_atomicity`): the
  fixture's "interrupt doesn't fire in time" FIXME was a misdiagnosis —
  the handler never stopped SysTick, so the free-running tick re-latched
  its pending bit and the NVIC tail-chained extra entries (nondeterministic
  Counter). The handler now stops the timer on first entry and guards the
  one tail-chained re-entry via an MMIO scratch cell (same-priority
  exceptions never nest), making `Counter == 2` deterministic; the
  `requires = ["QemuInterruptAxis"]` opt-out gate (a capability no target
  advertises) is removed and the interrupt axis actually executes on ARM.
- **Static ARM image packing** (`tyu build --mode=static
  --target=armv7m-unknown-none`): intra-module calls
  (`R_ARM_THM_CALL`/`R_ARM_THM_JUMP24`) pre-resolve at pack time via the
  shared Thumb-2 branch encoder (`lmod::reloc::thumb`) instead of failing
  with "unsupported internal relocation type 10". The shared encoder also
  fixes two latent loader bugs: the J1/J2 bits were pre-set in the BL
  second halfword (wrong for negative displacements), and the decoder
  sign-extended after doubling (negative displacements decoded wrong —
  both untested paths).
- **Pack lint schema**: kind-aware rules (metal packs require startup +
  linker script; hosted packs must not declare a linker script; an
  engine-hosted pack may declare no native units), the wasm pack's real
  `expected_abi_hash` (`0xada0c3a9a848bee9`; the declared zero was a
  masked lint failure), a `web-bundle` deploy method for the browser pack,
  and sysroot-routed capability glue for the hosted bundle. The three
  undiscovered `platforms/*/metal/platform.toml` duplicates are removed.

### Added

- `crates/tyu/tests/platform_kind.rs`: the declared-kind build matrix —
  hosted pack build + native run, hosted/wasm/bare dynamic-mode gates,
  metal dynamic firmware link, kind-aware lint, `platform list` kind
  reporting.
- `ci/guards.sh` G14 now also pins the kind declaration on every pack
  manifest.

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