# Target Capability Tiering — superseded

> **Status:** Superseded by the `Feature` / `PlatformCapability` split
> (see `devdocs/plans/multiplatform-parity-and-feature-gating.md`)
>
> **Date:** 2026-06-10
> **Scope owner:** compiler/runtime

## What happened

The original architecture had a single `PlatformCapability` enum that was
used both to describe what a target ISA could lower AND what a shipped
image included.  In practice this conflated two different concerns:

| Concern | Before | After |
|---|---|---|
| *Can this backend lower op X?* | `UnsupportedOp` wildcard in ARM/RISC-V | Every backend lowers every `OpKind` (full parity, FR-1) |
| *Should this image include feature Y?* | Same `PlatformCapability` used for gating | `Feature` enum (Concurrency, ModuleLoading) gated per profile in `tyu.toml` |

The old approach meant ARM and RISC-V had ~35 vs ~20 implemented ops,
with the remaining 15 falling through to `CodegenError::UnsupportedOp`.
This was rationalized as "embedded resource tiering" but was actually
an implementation gap.

## Current architecture

### `PlatformCapability` (codegen-core)

Describes what a target's *sysroot* provides (not what the ISA can do).
Used by the test harness to filter fixtures by `requires = [...]`.

- `TaskScheduler` — cooperative or preemptive task runtime
- `DynamicAlloc` — heap / region allocator
- `Channels` — channel IPC present

### `Feature` (codegen-core)

Describes what a *build image* includes.  Selected per `[profile.<name>]`
in `tyu.toml`.  The gate is enforced at the semantic layer (before codegen).

- `Concurrency` — `task spawn` construct + `concurrency.asm` runtime unit
- `ModuleLoading` — loader/module-load constructs + `modload.asm` runtime unit

### Runtime units

Core (`runtime.asm`) is always linked.  Feature-specific units are
conditionally assembled and linked:

```
runtime/<triple>/
  runtime.asm        # always — boot, trap, diag, testio, BSS
  concurrency.asm    # iff feature `concurrency` — task scheduler
  modload.asm        # iff feature `module-loading` — modpack section
```

The `link.ld` for each target places all three sections; absent input
sections are tolerated (empty section = no space allocated).

## Reversal condition

If a genuinely capability-limited target is added (e.g. an 8-bit MCU
that cannot do 64-bit arithmetic), introduce a REAL capability axis
in `PlatformCapability` and gate ops at the semantic layer — but do
NOT return to per-backend `UnsupportedOp` wildcards.
