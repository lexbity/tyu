# Runtime Contract Specification v1 (Draft)

This document defines the **runtime contract** required by compiled programs.  
It is intentionally small: most semantics are enforced at **compile time**; the runtime is an **ABI + a few primitives + platform glue**.

This spec covers:
- what a target platform must provide (Linux, bare metal, FreeRTOS, exokernel/library-OS)
- required symbols / ABI (data stack, entry, traps)
- optional hooks for debugging/testing tooling

Bytecode VM/runtime is **not** covered here (separate spec).

---

## 1) Goals

- **Deterministic execution**: no GC, no hidden allocations, predictable timing.
- **Portable core semantics**: the same source should compile for Linux simulation and embedded targets.
- **Tiny runtime**: runtime is minimal and target-specific; most checks are compile-time.
- **Tool-friendly**: consistent hooks for tracing, debug info, and test harnesses.

Non-goals (v1):
- full reflection / dynamic loading
- full OS abstraction layer baked into the language
- mandated scheduler or threading model

---

## 2) Definitions

- **Target**: an architecture + environment combo (e.g., `linux-x86_64`, `linux-armv6`, `esp32-xtensa-bare`, `esp32-xtensa-freertos`).
- **Platform runtime**: the per-target implementation of required symbols and services.
- **Core runtime ABI**: the minimal set of rules that all targets share.
- **Data stack**: the language evaluation stack (typed at compile time, stored in memory at runtime).
- **Checks**: runtime-inserted validations for contracts/subtypes when enabled by compiler flags.

---

## 3) Targeting model

The compiler targets a triple-like identifier:

Examples:
- `linux-x86_64-hosted`
- `linux-armv6-hosted` (Pi Zero)
- `esp32-xtensa-baremetal`
- `esp32-xtensa-freertos`
- `esp32-riscv-baremetal`

A target defines:
- CPU/ABI (calling convention, registers, endianness, alignment)
- entry model (hosted process vs reset vector)
- availability of services (time, IO, allocator, atomics)
- interrupt model (exists/doesn't exist; ISR ABI)

### 3.1 Profiles (compiler-side, not language)
Typical build profiles influence runtime requirements:
- `--checks=off|contracts|all`
- `--allow-raw-casts` (gate raw pointer casts)
- `-g` (emit debug metadata and enable trap-with-location)

---

## 4) Core runtime ABI (required on all targets)

### 4.1 Required exported symbols

Every final linked image must provide (directly or via platform glue):

- `main : ( -- )`  
  Program entrypoint in language terms.

Runtime must also provide:

- `__lang_start()`  
  Entry glue that initializes runtime state then calls `main`.  
  - On hosted targets, this may be called by `crt0`/`main()` wrapper.
  - On bare metal, this is reached from reset handler.

- `__lang_trap(code: u32) -> !`  
  Non-returning trap handler used by inserted checks and compiler assertions.

Optional (enabled by `-g` or by platform choice):
- `__lang_trap_loc(code: u32, file_id: u32, line: u32, word_id: u32) -> !`

### 4.2 Data stack requirements

The runtime must provide storage for the **data stack**.

Contract:
- The compiler assumes a contiguous memory region:
  - base address `DS_BASE`
  - limit address `DS_LIMIT`
  - current pointer `DS_PTR` (grows in a target-defined direction)

v1 requirement:
- single data stack for single-threaded programs
- multi-stack support is optional (see §8)

### 4.3 Data stack ABI (for native AOT)

The native backend uses the data stack for all operand passing.

Minimum requirements:
- alignment: at least `alignof(usize)`
- slot size: target “cell” size (typically `usize`)
- typed values may occupy 1+ cells (compiler knows layout)

Implementation detail:
- DS_PTR may be held in a dedicated register or in memory; this is target/backend-defined.

---

## 5) Checks and traps

### 5.1 Contract checks
When `--checks=contracts|all`:
- `requires` predicates are evaluated at word entry
- `ensures` predicates at word exit
- failure calls `__lang_trap(_CONTRACT_FAIL)` (or `_CONTRACT_FAIL_LOC` variant if available)

### 5.2 Subtype checks
When checks enabled (default):
- conversions to subtypes and subtype-typed parameters/returns are validated
- failures call `__lang_trap(_SUBTYPE_FAIL)`

### 5.3 Trap codes
Trap codes are stable constants in `core` (or a reserved range):
- `_CONTRACT_FAIL`
- `_SUBTYPE_FAIL`
- `_ASSERT_FAIL`
- `_UNREACHABLE`
- `_STACK_OVERFLOW` (optional)

Platform runtime decides what “trap” does:
- halt
- reset
- break into debugger
- log and halt

---

## 6) Platform service surface (sysroot contract)

The language’s `core` is target-independent. Platform-specific capabilities come from the platform sysroot package.

At minimum, platforms should define:

### 6.1 `platform.startup`
- hosted: wrapper that calls `__lang_start()`
- bare metal: reset handler, vector table, memory init, then `__lang_start()`

### 6.2 `platform.mmio` (if not inlined)
- volatile load/store primitives (or compiler lowers directly to instructions)
- memory barriers/fences

### 6.3 `platform.critical` / `platform.atomic`
- minimal critical-section entry/exit (for `lock` and ISR-safe operations)
- optional atomic ops

Hosted mapping:
- pthread mutex / atomics

Bare metal mapping:
- interrupt mask manipulation (CPU-specific)

### 6.4 `platform.mem` (optional)
Region allocator support (if platform allows allocation):
- `region-create`, `region-alloc`, `region-reset`, `region-destroy`

If a platform forbids allocation:
- `platform.mem` is absent or its symbols are unavailable, causing link failure if referenced.

ABI notes (v1.3 baseline):
- `Region` is an opaque handle (size: one machine word).
- `RegionRef` / `RegionRefMut` are non-owning borrowed handles (also one word).
- `Slice(T)` / `SliceMut(T)` are ABI structs:
  - `ptr : ^T` (u64 on 64-bit targets)
  - `len : usize`
  - total size: two machine words

### 6.5 `platform.time` (optional)
- monotonic clock
- sleep/delay helpers
- (optional) timer interrupt hookup

### 6.6 `platform.io` (optional)
- logging / UART / stdout mapping
- file/network only for hosted targets


### 6.7 `platform.task` (optional)
If provided, exposes cooperative and/or preemptive tasks:
- `spawn`
- `yield` (suspending)
- `join` (optional)
- `sleep_*` (optional)
- `run` (optional handler/driver for `{suspend}` quotations)

ABI notes (v1.3 baseline):
- `Task` is an opaque handle (size: one machine word).

### 6.8 `platform.channel` (optional)
If provided, exposes message passing primitives used by `<|` / `|>`:
- `create`
- `send`
- `recv`
- (optional) `try_send` / `try_recv`

---

## 7) Interrupts and ISR ABI

ISR support is target-specific.

### 7.1 Attribute-driven ISR
Words marked with `@ISR(...)` must be compiled to the target’s ISR ABI:
- correct prologue/epilogue
- correct return instruction (e.g., `iret` / `mret` / target-specific)
- correct symbol naming/placement if required by vector table

### 7.2 ISR safety rules (compile-time)
Within `@ISR` words (and any word they call if flagged ISR-safe):
- no allocation (no `platform.mem`)
- no blocking calls
- only call ISR-safe words
- `lock` may be restricted or mapped to IRQ masking rules

Runtime requirement:
- platform must provide any vector table glue required to route interrupts to `@ISR` symbols.

---

## 8) Concurrency and scheduling (optional; platform-provided)

The language does not mandate a scheduler. The runtime contract defines optional hooks that enable:
- cooperative suspension (`suspend` effect)
- multi-task execution
- message passing via channels (actors can be built on top)

### 8.1 Single-thread baseline (v1)
All targets must support:
- single-threaded execution with one data stack

A target may still implement `platform.task`/channels in a single-threaded “run loop” style.

### 8.2 Suspension (`suspend` effect)

If the sysroot exposes any suspending operations (e.g. `platform.task.yield`, `platform.task.sleep_*`), the compiler may mark callers with effect `{suspend}`.

Runtime requirement:
- suspension points must be able to transfer control back to a scheduler/driver.

Implementation options (platform/compiler choice):
- **stackless** lowering (state machines): no special runtime beyond `yield`/polling
- **stackful** coroutines: requires stack switching + saving/restoring register state

If suspending code must be callable from non-suspending code, provide a handler/driver:
- `platform.task.run : ( Quot!{suspend} -- )` (drive until completion; exact signature may vary by ABI)

### 8.3 Optional multi-task model
A platform may provide:
- per-task data stack allocation/init
- context switch glue (save/restore DS_PTR and registers)
- task creation/join

This is platform-defined:
- Linux: threads/pthreads
- RTOS: tasks
- bare metal: cooperative “tasks” as state machines

If supported, the sysroot should expose:
- `platform.task.spawn`
- `platform.task.yield` (suspending)
- `platform.task.join` (optional)
- `platform.task.sleep_*` (optional)

### 8.4 Channels (message passing)

If channels are supported, the sysroot should expose a module like:
- `platform.channel`

Recommended minimal surface:
- `platform.channel.create : ( capacity -- |T| )` (or untyped channel with typed wrappers in stdlib)
- `platform.channel.send   : ( |T| T -- )` (may block or return a status; platform-defined)
- `platform.channel.recv   : ( |T| -- T )` (may block or return an option/status; platform-defined)

The language’s `<|` / `|>` operators lower to these calls (or stdlib wrappers).

Notes:
- Lock-free vs locked queues, blocking vs polled receive, and memory strategy (copy vs zero-copy for `iso`) are platform-defined.
- The compiler enforces move-only rules for `iso` payloads; the runtime does not need to track ownership.

### 8.5 `lock` mapping under concurrency
`lock` must map to a platform-defined critical section strategy:
- bare metal: interrupt masking / priority ceiling
- RTOS: critical sections / mutex
- Linux: mutex/atomic

The compiler assumes `lock` is atomic, non-suspending, and used for short critical sections (typically MMIO/devices). Fairness is not assumed.

---
## 9) Exokernel / library-OS considerations

If the runtime is the basis of a library-OS (exokernel style), the platform layer becomes the “OS personality”.

Additional aspects to define (beyond baseline targets):

### 9.1 Capability model
Exokernel systems commonly require explicit capabilities for:
- memory regions
- devices
- interrupts
- scheduling/CPU time

Recommendation:
- represent capabilities as opaque types in `platform.cap`
- only platform code can mint them
- user code passes capabilities explicitly (no ambient authority)

### 9.2 Device model / drivers
Define a convention:
- MMIO register-maps live in platform packages
- drivers expose `.def` interfaces (portable) with platform `.mod` implementations
- DMA / buffer pinning is explicit (often capability-gated)

### 9.3 Memory ownership boundaries
If you later add stronger ownership, the exokernel layer should define:
- which regions are user-owned vs kernel-owned
- how physical memory is carved into regions
- how region destruction interacts with device DMA ownership

### 9.4 Scheduling hooks
If the platform provides scheduling:
- define minimal hooks: `yield`, `spawn`, `set_priority`, `sleep_until`
- define which are ISR-safe

---

## 10) Testing and debugging tool interaction

The runtime should support a small set of hooks that make testing/debugging easy across targets.

### 10.1 Hosted testing (Linux simulation)
Recommended:
- `lang test` runs test entrypoints under hosted runtime
- contracts/subtype checks enabled by default
- trap uses `SIGTRAP` or process abort so CI catches failures
- optional trace log sink

### 10.2 Trace hooks (portable)
Provide weak symbols (platform may override):
- `__lang_trace_event(id: u32, a: usize, b: usize)`
- `__lang_trace_word_enter(word_id: u32)`
- `__lang_trace_word_exit(word_id: u32)`

Compiler can emit these under a `-Z trace` style flag.

### 10.3 Debugger integration
Hosted:
- symbols per word enable backtraces
- optional `__lang_trap_loc` improves error reporting

Embedded:
- optional semihosting/UART logging
- optional GDB stub in platform runtime (not required by v1, but strongly recommended)

### 10.4 Deterministic replay (optional future)
If you want “same test on Linux and ESP32”:
- standardize time and randomness behind `platform.time` / `platform.rand`
- allow injecting deterministic providers during tests

---

## 11) Packaging and sysroot layout

A toolchain ships:
- `core` package (language-level)
- one or more `platform/*` packages, each implementing this runtime contract

The build tool selects exactly one platform sysroot per target.

---

## 12) Minimal v1 compliance checklist

A platform runtime is v1-compliant if it provides:

Required:
- `__lang_start()`
- `__lang_trap(code)`
- data stack storage + initialization
- ability to call language `main`

Recommended:
- `__lang_trap_loc(...)` when `-g`
- `platform.critical` for `lock`
- `platform.mmio` or inline lowering support
- basic logging (`platform.io.log`) for embedded bring-up

Optional:
- `platform.mem` (regions)
- ISR support (`@ISR` ABI)
- tasks/threads (`platform.task`)
- channels (`platform.channel`)
- suspension handler/driver (`platform.task.run`) if `{suspend}` is used
- trace hooks and/or GDB stub
