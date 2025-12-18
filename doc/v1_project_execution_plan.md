# v1 Project Execution Plan (Implementation)

This document describes a detailed execution plan to implement the v1 specifications:
- `doc/language_v1_reference_manual_updated5.md`
- `doc/compiler_technical_spec_v1_updated2.md`
- `doc/runtime_contract_spec_v1_updated2.md`
and the engineering decisions captured in:
- `doc/v1_project_decisions.md`

The plan is organized as **vertical milestones** (each produces an end-to-end, testable deliverable) while keeping the codebase split by **stable subsystem boundaries** (frontend/semantics/IR/codegen/runtime/sysroot/tools).

---

## Guiding principles

- **Always runnable on Linux** as early as possible (compile → assemble → link → run).
- **Semantics shared across targets**: front-end + checks + IR are target-independent; targets differ mainly in `codegen` + `runtime/sysroot`.
- **Checks on by default** (subtypes + contracts), with explicit flags to relax (`--checks=off|contracts|all`).
- **No “internal compiler error”**: invalid programs must report diagnostics, not panic/abort.
- **Testing-heavy**: every milestone adds tests at multiple levels (unit, snapshot, integration, and later on-hardware smoke).

---

## Non-negotiable constraints (locked)

- Initial target: `linux-x86_64-hosted`.
- Future targets: `esp32-*` (native firmware), `linux-armv6-hosted` (Pi Zero).
- `langc` and `lang-assemble` are `#![no_std]` (Linux hosted may use `alloc` + `libc`, but not `std`).
- Runtime + sysroot packages are `#![no_std]` (Linux hosted may use `alloc` + `libc`).
- Native codegen path: emit Flat Assembler (FASM) assembly for ELF64 (`--emit=asm`), assembled/linked by `lang-assemble`.
- Control-flow and safety primitives are compiler-recognized intrinsics per specs (`if/while/loop/lock/return`, `dup/drop`, borrow forms).

---

## Workstreams (stable subsystem boundaries)

These are not separate teams; they are “modules of work” that can be developed incrementally.

### A) Tooling and CLI surface (`langc`, `lang-assemble`)

- `langc`: driver, flags, sysroot selection, module resolution, diagnostic formatting, dump formats.
- `lang-assemble`: Linux hosted tool that runs `fasm` + `ld`/`cc`, manages artifacts, and returns deterministic errors.

### B) Front-end (lexer/parser/module loader)

- Lexing tokens and punctuation per specs.
- Parsing to AST: declarations + terms + quotations + operators.
- Module graph + import resolution + `.def`/`.mod` conformance checking.

### C) Semantics (name resolution, type/effect checking, intrinsics typing)

- Typed stack checker for words and quotations.
- Intrinsics typing rules pinned in compiler spec.
- Quotation typing rules (immediate inference, escaping requires `( -- )` annotation, optional `!{suspend}`).
- Effect propagation (`{}` vs `{suspend}`) and restrictions.

### D) Safety and static checks (subtypes, contracts, place/borrow checking)

- Subtype checks insertion points (casts + word boundaries) and compile-time constant validation.
- Contracts (`requires/ensures`) insertion and trap mapping; ISR safety constraints for `@ISR`.
- Place model + borrowing checks including scoped borrows and suspension liveness.

### E) IR and verification

- Typed stack IR + basic blocks representation.
- IR verifier (invariants: stack balance, type soundness at block edges, effect constraints).
- Dump formats: readable `.lir` and/or `.json`.

### F) Backends and runtime contract integration

- Linux x86_64: FASM emission + runtime ABI integration (DS registers/symbols, traps, entry).
- Future: embedded runtime contract surfaces; later `linux-armv6-hosted`.

### G) Sysroot packages (core + platform)

- `core`: target-independent language-level package (trap codes, basic ops, declarations).
- `platform/linux`: hosted runtime bindings (`platform.startup`, `platform.io.log`, etc.).
- Later `platform/esp32`, `platform/pi0`.

---

## Milestones (vertical, end-to-end)

Each milestone lists:
- **Deliverable**: what we can run/observe.
- **Implementation**: main modules to build.
- **Tests**: what must exist before moving on.

### Milestone 0 — Repository skeleton + “no_std hosted tools” foundation

Deliverable:
- `langc --help` and `lang-assemble --help` run on Linux with no `std`.

Implementation:
- Create common “hosted-no_std” support crate for Linux (thin wrappers over `libc` for:
  - file reads/writes
  - directory iteration
  - process spawn/exec
  - environment variables
  - basic time if needed for tests/logging)
- Decide allocator strategy for hosted tools (`alloc` required): minimal global allocator for Linux (e.g., `mmap`/`brk`-backed).
- Define a diagnostic format (single-line + optional spans) that is stable for snapshots.

Tests:
- Unit tests for foundational utilities (in a separate hosted test harness if needed).
- “Smoke” tests that run `langc` and `lang-assemble` and validate exit codes and stdout format.

Acceptance checklist (Milestone 0):
- **Crates and binaries exist**
  - `langc` binary crate: `#![no_std]` with `extern crate alloc` allowed; depends on `libc` (Linux hosted) but not `std`.
  - `lang-assemble` binary crate: same `no_std` policy as `langc`.
  - `lang_host` (or similarly named) support crate providing a minimal Linux interface surface for both tools.
- **Minimum CLI contract (stable)**
  - `langc --help` prints usage and exits `0`.
  - `langc --version` prints a semver-ish version string and exits `0`.
  - `lang-assemble --help` prints usage and exits `0`.
  - `lang-assemble --version` prints a semver-ish version string and exits `0`.
  - All usage errors exit non-zero and print a single-line error prefix (e.g. `error:`) suitable for snapshot testing.
- **Minimum file/layout assumptions**
  - Sysroot root is discoverable via one of:
    - `--sysroot <path>` (preferred), or
    - `LANG_SYSROOT=<path>` env var.
  - Output directory is controllable via `--out-dir <path>` (create if missing).
- **Minimum “hosted-no_std” OS surface**
  - File IO: open/read/write/close; atomic replace via write-to-temp + rename.
  - Directory: list entries for module discovery (or a simpler “explicit file list only” mode to start).
  - Process: execute external tools with args and capture exit code (needed for `lang-assemble`).
  - Environment: read env vars for `--sysroot` defaulting and tool paths.
  - Error: convert `errno` to deterministic diagnostics (no locale-dependent strings in snapshots).
- **Allocator policy**
  - A single global allocator exists for hosted tools (mmap/brk-backed is fine) and is exercised by a basic smoke path (e.g., reading a file into a buffer).
- **CI/test harness**
  - A hosted test runner exists (may use `std`) that can invoke `langc`/`lang-assemble` and snapshot stdout/stderr in a deterministic way.

### Milestone 1 — Parse-only pipeline with module units

Deliverable:
- `langc file.mod --emit=ast` produces an AST dump (or JSON) for a simple module.

Implementation:
- Lexer + parser for:
  - compilation units (`module`, `import`, `export`, `end`)
  - declarations: `word`, `type`, `subtype`, `struct`, `enum`, `const`, `resource`, `register-map`
  - terms: literals, word refs, quotations, casts (`as/as?/bitcast`), borrow tokens, pipe ops (`<|`/`|>`)
- Module loader with include paths and sysroot roots.

Tests:
- Golden parse tests (AST) for representative syntax.
- Parser fuzzing (start here) with “no panic” invariant.
- Snapshot tests for parse errors (bad tokens, mismatched delimiters).

### Milestone 2 — `.def`/`.mod` interface conformance and import resolution

Deliverable:
- `langc` can load multi-module programs and enforce `.def` matching.

Implementation:
- Symbol tables for exported declarations; `.def` vs `.mod` comparison rules (names, kinds, signatures/effects, ABI attributes).
- Import binder: `import M { a b c }` binds those names; errors are deterministic.

Tests:
- Fixtures for valid/invalid interface matching.
- Snapshot diagnostics for missing export, wrong signature, wrong attribute, etc.

### Milestone 3 — Typed stack checker MVP + core intrinsics typing

Deliverable:
- `langc --emit=ir` produces typed IR for simple programs using arithmetic, locals, and control intrinsics; compile-fail errors are precise.

Implementation:
- Typed stack checker:
  - literals push types
  - word calls pop/push by signature
  - `=> name` binds immutable locals
  - declared stack effects `( ... -- ... )` are validated
- Intrinsics typing:
  - `if`: branch stack equivalence
  - `while`/`loop`: stack-preserving rules
  - `return`: allowed only with explicit stack-effect declaration
  - `lock`: `( S -- S )` and `{}` only, no nesting
- Quotations:
  - immediate quotations inferred where allowed
  - escaping quotations require explicit stack effect annotation, optional `!{suspend}`
  - no implicit capture rule enforced

Tests:
- Compile-fail snapshots for stack drift, type mismatch, illegal return, illegal lock nesting.
- Golden IR dumps for canonical examples.

### Milestone 4 — Subtypes + contracts end-to-end (checks inserted by default)

Deliverable:
- Running binaries on Linux trap when contracts/subtype checks fail; `--checks=off` changes behavior.

Implementation:
- Parse and represent:
  - `subtype` declarations with ranges
  - word `requires` / `ensures`
- Insert checks per compiler spec:
  - subtype checks on `as?/as` and word boundaries (params/returns)
  - contract predicates at entry/exit
- Map failures to runtime trap codes (`__lang_trap(code)` and later `__lang_trap_loc` for `-g`).

Tests:
- Integration tests that compile+assemble+link+run on Linux and assert:
  - checks are on by default
  - `--checks=contracts` vs `--checks=all` differences
  - `--checks=off` removes checks (behavioral test + IR-level assertion)

### Milestone 5 — Place/borrow checking + suspension liveness rules

Deliverable:
- Compiler enforces borrow/place restrictions, scoped borrows, non-escape, and suspend liveness (even if `{suspend}` is rare initially).

Implementation:
- Place model:
  - locals, fields-of-places, MMIO regs, resource places (inside `lock`)
- Borrow operators:
  - `&place` → `^T`, `&!place` → `^!T`, only on places
- Scoped borrows:
  - `&[ ... ]` and `&![ ... ]` for Arrays → slices and Regions → borrowed handles
  - non-escape enforcement (return/store/capture/opaque call)
- Suspension liveness:
  - define effect sets on words; `{suspend}` boundaries reject live scoped-borrow values
  - enforce “no suspend inside `lock`” and “no suspend inside `&![ ... ]`” per spec policy

Tests:
- Snapshot suite for each rejected pattern (escape routes + suspension violations).
- Positive tests for allowed patterns (borrow consumed before suspend).

### Milestone 6 — `register-map` legality + volatile lowering (MMIO)

Deliverable:
- End-to-end MMIO examples compile with correct volatile semantics and legality checks.

Implementation:
- Parse `register-map` declarations:
  - scalar regs, reg arrays, nested sub-block arrays, optional stride
  - bitfields and access modes (`ro/rw/wo/w1c/w1s/rc`) with reserved bits
- Lowering rules:
  - register places are addressable; fields are not
  - read lowering (load + mask/shift)
  - write lowering (RMW for `rw`, special write patterns for `w1c/w1s`)
- Ensure the IR has explicit “volatile load/store” ops so backends can preserve ordering.

Tests:
- Compile-fail legality tests (alignment, field address, access mode violations).
- Golden IR dumps showing volatile ops and correct masks/shifts.

### Milestone 7 — Linux x86_64 backend: runnable programs via FASM

Deliverable:
- `langc --emit=asm` + `lang-assemble` produce working Linux x86_64 executables.

Implementation:
- Runtime ABI integration (per `doc/v1_project_decisions.md`):
  - `r15 = DS_PTR`, `r14 = DS_LIMIT`
  - `__lang_start()` initializes DS region and calls `main`
  - `__lang_trap(code)` terminates predictably
- FASM emission:
  - function symbols per word
  - stack ops compiled against data stack memory
  - calls use normal `call`/`ret`
  - inserted checks call `__lang_trap`
- `lang-assemble` contract:
  - stable artifact naming and output directories
  - deterministic invocation of `fasm` and linker
  - stable error formatting for snapshots

Tests:
- End-to-end integration suite:
  - arithmetic/control/quotations
  - subtype/contract failures and successes
  - borrow/place constraints in generated code paths
  - MMIO lowering tests can be “compile-only” on Linux (or target a fake MMIO region in memory)

### Milestone 8 — Sysroot maturation and “core/ platform/linux” usability

Deliverable:
- A small standard set of packages allows writing non-trivial programs without reinventing primitives each test.

Implementation:
- Define minimal `core` conventions:
  - trap code constants
  - core types and canonical names
  - module structure and re-exports
- `platform/linux`:
  - `platform.io.log` writes to a file (configurable)
  - `platform.time` optional for tests
  - `platform.critical` minimal mapping for `lock` (even if single-thread baseline)

Tests:
- “Sysroot conformance” tests: sysroot package builds, exports stable surfaces, and matches `.def`/`.mod`.

### Milestone 9 — ESP32 bare-metal bring-up (native firmware, no scheduler yet)

Deliverable:
- Minimal firmware that boots, logs, and traps; compiler can emit code for the target (even if link steps evolve).

Implementation (recommended order):
- `platform/esp32` runtime:
  - `__lang_start`, `__lang_trap`, data stack storage/init
  - `platform.startup` reset/init
  - `platform.mmio`, `platform.critical`, `platform.io`
- Keep `{suspend}` unavailable initially (no `platform.task`).
- Add MMIO register-map definitions for real peripherals and small drivers behind `.def` interfaces.

Tests:
- Compile-only cross tests in CI for the target (no linking required to start).
- On-hardware smoke tests: boot + log + trap code verification.

### Milestone 10 — Pi Zero (`linux-armv6-hosted`) enablement

Deliverable:
- Hosted Linux programs run on armv6 with the same language semantics and tests adapted for the target.

Implementation:
- Add target triple and backend support (assembly dialect and ABI differences).
- Reuse the Linux hosted runtime model with armv6 specifics.

Tests:
- Compile-only CI plus optional QEMU run tests if feasible.

---

## Testing architecture (what we build and when)

### 1) Unit tests (Rust)

- Lexer edge cases and longest-match tokenization.
- Parser correctness and error recovery (if implemented).
- Resolver: import cycles, shadowing rules, attribute placement errors.
- Typed stack checker: stack state transitions and intrinsic rules.
- Borrow checker: place classification, escape detection, suspension liveness.
- IR verifier: invariants and type consistency.

### 2) Snapshot tests (compile-fail)

- A stable directory of `.mod` fixtures with expected diagnostics:
  - parse errors
  - resolution errors
  - type/stack errors
  - borrow/escape errors
  - contract/subtype insertion constraints
  - register-map legality errors

### 3) Golden tests (dumps)

- AST dumps for representative grammar forms.
- IR dumps showing inserted checks and volatile ops.
- Backend dumps: generated FASM for canonical small programs.

### 4) Integration tests (Linux)

- Compile → `--emit=asm` → `lang-assemble` → run:
  - exit code / stdout / log file content
  - trap behavior and codes

### 5) Fuzzing

- Start with parser fuzzing (no crashes).
- Extend to parse→typecheck fuzzing with “no panic, stable error”.

---

## Definition of Done (v1)

v1 is “done” when:
- Linux x86_64 hosted:
  - core language features in the reference manual are implemented (types, control intrinsics, locals, quotations with typing rules, borrowing rules, subtypes/contracts, register-map).
  - `langc` can compile a multi-module program with `.def`/`.mod` checking.
  - `lang-assemble` reliably produces runnable binaries from `--emit=asm`.
  - tests cover golden/snapshot/integration with high confidence.
- The runtime contract is honored:
  - traps are stable and deterministic
  - data stack contract is satisfied
- Embedded readiness:
  - compiler core is target-independent
  - a minimal `platform/esp32` bare-metal runtime is feasible without changing language semantics (even if not fully shipped yet).

---

## Early risk list (and how the plan mitigates it)

- **`no_std` hosted tooling friction**: mitigate by building a small `libc`-based platform layer early (Milestone 0) and keeping formats stable for tests.
- **Backend complexity**: mitigate by getting a typed IR and verifier before “real” codegen (Milestones 3–5).
- **Borrow/suspend interaction**: mitigate by enforcing effect sets and suspension liveness in the checker even before tasks exist (Milestone 5).
- **MMIO correctness**: mitigate by making volatile operations explicit in IR and verifying lowering via golden tests (Milestone 6).
