# Linux Hosted Target Profile (v1)

This document defines the **current implemented** “Linux hosted” execution model for Tyu on the repository’s hosted toolchain.

It is intentionally precise and tied to the current implementation so it can be used as a debugging and testing reference.

## Target identity

- Logical target: `linux-x86_64-hosted` (name used in docs; `--target` is currently a stub in `langc`)
- Output kind today: **FASM ELF64 executable** (not relocatable `.o`)
  - Implemented in `crates/langc/src/main.rs` (asm emitter) and assembled via `lang-assemble` (wrapper around `fasm`)

## Process entry and calling convention

- Executable entrypoint symbol: `__lang_start`
- `__lang_start` initializes the language data stack registers, then calls the Tyu word `main`.
- Each Tyu word is compiled to a native symbol (FASM label) and called with a normal `call`.
- Return from `main`:
  - If `main` has at least one output value, the compiler treats the top-of-stack value as the process exit code (masked to 8 bits).
  - If `main` has no outputs, exit code is `0`.

Implementation: `crates/langc/src/main.rs` (`IrAsmGen::emit_prelude`).

## Data stack ABI (hosted v1)

The hosted backend uses a dedicated “data stack” in memory for all operand passing.

- `DS_PTR` register: `r15`
  - Grows upward (push = store at `[r15]` then `add r15, 8`)
- `DS_LIMIT` register: `r14`
- Storage:
  - `__lang_ds_base rb 65536` (64 KiB)
  - `__lang_ds_limit:` label marks the end of the allocated region
- Stack overflow checks:
  - push paths check `r15 + 8 > r14` and trap on overflow.

Implementation: `crates/langc/src/main.rs` (`emit_push_i64`, `emit_push_rax`, `emit_load_local`, `emit_prelude`).

## Trap behavior (hosted v1)

- Trap symbol: `__lang_trap`
- Trap mechanism: Linux `exit(2)` syscall (`rax=60`) with code in `rdi`.
- Trap codes currently match `sysroot/Core.mod`:
  - `_STACK_OVERFLOW = 10`
  - `_CONTRACT_FAIL  = 20`
  - `_SUBTYPE_FAIL   = 21`
  - `_ASSERT_FAIL    = 22`
  - `_UNREACHABLE    = 23`

Notes:
- There is no `__lang_trap_loc` implementation in the hosted backend yet.
- Many “not implemented” backend paths trap with `_UNREACHABLE`.

Implementation: `crates/langc/src/main.rs` (trap emission) and `sysroot/Core.mod`.

## Sysroot expectations (hosted v1)

The compiler expects sysroot modules to exist for imported platform surfaces.

In this repo:
- `sysroot/Core.def` / `sysroot/Core.mod` define trap code constants.
- `sysroot/platform/linux.def` / `sysroot/platform/linux.mod` define hosted platform surfaces.
- `sysroot/platform/channel.def` / `sysroot/platform/channel.mod` declares channel ops for the language surface.

Important hosted-specific behavior:
- `platform.task.yield` is declared `{suspend}` but is a **no-op** in the hosted backend today.
- `platform.critical.enter/exit` are **no-ops** today (no real mutual exclusion on hosted yet).
- `platform.channel.*` is not implemented as a linked runtime; it is special-cased in the hosted asm backend with a fixed-size ring buffer.

Implementation: `sysroot/platform/linux.mod`, `sysroot/platform/channel.mod`, and `crates/langc/src/main.rs` (call special-casing).

## Validation / how to test hosted v1

Prerequisites:
- `fasm` available on PATH (used by `lang-assemble`)

Quick check:
- Run the integration test that compiles, assembles, and executes a tiny program:
  - `crates/tooling-tests/tests/smoke.rs` (`milestone7_compile_assemble_run_exit_code`)

