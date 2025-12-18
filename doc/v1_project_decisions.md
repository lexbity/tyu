# v1 Project Decisions (Living Document)

This document records engineering decisions that constrain implementation across the compiler, runtime, sysroot, and tooling.

## Scope

- Targets (initial): `linux-x86_64-hosted`, then `esp32-*`, then `linux-armv6-hosted` (Pi Zero).
- Compiler implementation language: Rust.
- Runtime/sysroot: per-target, `no_std`.

---

## Rust `no_std` policy

- `langc` (compiler driver) is `#![no_std]`.
  - Linux-hosted build may use `alloc` and the `libc` crate for OS interfacing (files, directories, subprocess, etc.).
  - No dependency on Rust `std` in `langc`.
- Target runtimes and sysroots are `#![no_std]`.
  - Linux hosted runtime may use `alloc` + `libc` (but not `std`).

Testing tooling:
- We may use a separate hosted test runner binary (can use `std`) to orchestrate snapshot diffs and execute produced binaries, while ensuring the runtime-under-test remains `no_std`.

---

## Native ABI: data stack (Linux x86_64 SysV)

### Register assignment

- `DS_PTR` is held in `r15` (callee-saved).
- `DS_LIMIT` is held in `r14` (callee-saved).
- `DS_BASE` is used only for initialization and/or diagnostics (not required in a register for fast paths).

### Growth direction and pointer meaning

- Data stack grows **upwards** (toward higher addresses).
- `DS_PTR` points to the **next free** byte (top-of-stack + 1).

This makes push/pop patterns consistent:
- push `n` bytes: bounds-check `DS_PTR + n <= DS_LIMIT`, store at `[DS_PTR]`, then `DS_PTR += n`
- pop `n` bytes: `DS_PTR -= n`, then load from `[DS_PTR]`

### Runtime-provided symbols

The Linux runtime provides (linker-visible):
- `__lang_ds_base : usize` (address of the first byte of the data stack region)
- `__lang_ds_limit : usize` (address one-past-the-end of the data stack region)

And initializes `r15 = __lang_ds_base`, `r14 = __lang_ds_limit` before calling language `main`.

### Stack overflow

- v1: bounds checks in compiler-generated code trap with `_STACK_OVERFLOW` (optional code per runtime spec).
- Hosted improvement (later): allocate the data stack via `mmap` and use a guard page; still keep `__lang_ds_base/__lang_ds_limit` as the contract.

---

## Assembly/object pipeline (Linux x86_64)

- `langc --emit=asm` outputs Flat Assembler-compatible assembly for ELF64.
- Assembling/linking is a platform step:
  - `lang-assemble` (platform tool) invokes `fasm` and then `ld`/`cc` to produce an executable.
  - `lang-assemble` is also `#![no_std]` (Linux-hosted may use `alloc` + `libc`, but not `std`).
  - `langc` may remain “asm-only” initially; adding `--emit=obj` can come later once the `.o` pipeline is stable.

---

## Module model (runtime)

- Modules are resolved and linked at **compile time** (no runtime/dynamic module loading in v1).
- `.def`/`.mod` interface checking is enforced by the compiler as specified.

---

## Intrinsics

Treated as compiler-recognized intrinsics (at least in IR):
- Control/scoping: `if`, `while`, `loop`, `lock`, `return`
- Core stack ops: `dup`, `drop` (needed to enforce `iso` move-only and destructor rules)
- Borrow/scoped borrow syntax: `&place`, `&!place`, `&[ ... ]`, `&![ ... ]`

---

## Logging and trace hooks

- Each platform runtime/sysroot defines `platform.io.log` as the canonical logging surface.
- Hosted Linux runtime writes logs to a file (exact path/config TBD by platform package), to support deterministic test runs.
- Optional runtime trace hooks from the runtime contract remain available for compiler-driven tracing.

---

## Embedded “libraryOS / exokernel” direction (ESP32, Pi Pico-class)

Recommended v1 approach based on the current specifications:
- Start with a **bare-metal profile** first:
  - implement only the required runtime ABI (`__lang_start`, `__lang_trap`, data stack)
  - implement `platform.startup`, `platform.mmio`, `platform.critical`, `platform.io`
  - omit `platform.task`/`platform.channel` initially (so `{suspend}` is simply unavailable)
- Add exokernel/libraryOS capabilities incrementally (runtime contract §9 guidance):
  - define `platform.cap` as opaque capability types
  - keep MMIO maps and device drivers in platform packages with `.def` interfaces
  - define region/memory ownership boundaries explicitly before enabling allocation-heavy features
  - add scheduling hooks (`yield`, `spawn`, `sleep_until`) only when we can fully enforce `{suspend}` + borrow liveness rules end-to-end

---

## Versioning and layout

- Follow Linux-kernel-like conventions for versioning and tree layout where sensible.
- Sysroot layout remains stable and versioned alongside the toolchain.
