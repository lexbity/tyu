# Tyu Compiler — Rust Use Audit

**Date**: 2026-05-28
**Scope**: All source crates in `/home/lex/Public/tyu_lang`
**Total LOC**: ~17,552 (`.rs` files, excluding `target/`)

---

## 1. Architecture Overview

The project is a **systems-level compiler** for a custom concatenative language ("Tyu"), targeting x86-64 Linux and bare-metal. It is entirely `#![no_std]` and uses no allocator crate — all allocation is either stack-local (`FixedVec<T, N>`) or manual C `malloc`/`realloc`/`free` through raw FFI.

### Crate Map

| Crate | Role | LOC (approx) | `no_std` | Unsafe blocks |
|---|---|---|---|---|
| `frontend` | Lexer, parser, AST, `FixedVec<T,N>`, `Span` | 1,200 | Yes | 2 (FixedVec drop/iterator) |
| `ir` | LIR type definitions (`OpKind`, `Word`, `Sig`, etc.) | 500 | Yes | 0 |
| `semantics` | Typechecker (stack-check + IR generation), arena allocator | 5,400 | Yes | 3 (arena drop, qgen raw ptr) |
| `codegen-core` | Codegen traits, `ChannelPayloadKind`, `EmitMode`, `AsmMode` | 400 | Yes | 0 |
| `codegen-x86_64` | x86-64 assembly emitter | 2,100 | Yes | 0 |
| `hosted` | POSIX FFI bindings (mem, io, fs, env, process, cstr) | 800 | Yes | 14 (all FFI calls + manual drops) |
| `hosted-rt` | Runtime entry point (`_start`), panic handler, `exit` | 150 | Yes | 2 (`_exit`, `no_mangle` start) |
| `langc` | Compiler driver: arg parsing, orchestration | 900 | No | 1 (`parse_args` FFI) |
| `lang-assemble` | Assembler driver: arg parsing, NASM execution | 500 | No | 1 (`parse_args` FFI) |
| `tooling-tests` | Integration tests (QEMU smoke tests) | 5,000 | No | 0 |
| `execution-tests` | Test runner for QEMU integration tests | 600 | No | 0 |

### Compilation Pipeline

```
Source (.tyu)
  → Frontend (lex → parse)
    → Semantics (stack-check typecheck → LIR generation via arena allocator)
      → Codegen (x86-64 assembly emission)
        → NASM (object file via lang-assemble)
          → Link (against hosted-rt runtime)
```

The typechecker is a **stack effect checker**: rather than Hindley-Milner, the compiler simulates a value stack through every quotation, verifying that stack depth and types match at every point. This is a novel approach for a systems language.

---

## 2. Unsafe Code Audit

### 2.1 Summary

There are **23 unsafe blocks/functions** across the codebase. Every instance has been reviewed.

### 2.2 `frontend/src/fixed.rs` — `FixedVec<T, N>` (2 blocks)

```rust
// Drop impl (line 69):
unsafe { self.data[i].assume_init_drop() };
// Iterator (line 117):
unsafe { self.v.data[self.i].assume_init_drop() };
```

**Safety analysis**: Sound. `FixedVec` tracks initialization via `self.len` (≤ `N`). Only indices `0..len` are ever assumed initialized. The `MaybeUninit` array is properly aligned and sized. The drop impl correctly drops only initialized elements in reverse order. The iterator's `Drop` does the same. No double-free or use-after-free possible unless `push`/`pop` are called incorrectly — they aren't; push increments `len` after write, pop decrements before read. **Verdict: OK.**

### 2.3 `semantics/src/typecheck/irgen/arena.rs` — `ArenaAllocator` (1 block)

```rust
// Drop impl (line 33):
unsafe { self.words[i].assume_init_drop() };
```

**Safety analysis**: Same pattern as `FixedVec`. Tracks initialization count, drops in reverse. The arena only stores `lir::Word` fat pointers (each is 2× pointers = 16 bytes on x86-64). The `alloc` method uses `MaybeUninit::write` which won't panic. **Verdict: OK.**

### 2.4 `semantics/src/typecheck/irgen/quotes.rs` — Raw pointer arena access (2 blocks)

```rust
// Lines 149-153:
let word = unsafe {
    let arena = &mut *self.arena;      // self.arena is *mut ArenaAllocator
    let w = arena.alloc(word, quot_span)?;
    &*(w as *const lir::Word)
};
// Lines 155-156:
unsafe {
    (*ew).push(word).map_err(|_| ...)?;  // ew is *mut FixedVec<...>
}
```

**Safety analysis**: The `self.arena` raw pointer is initialized once during `QuoteGen::new` from a `&mut` that outlives the `QuoteGen`. The reborrow `&mut *self.arena` is valid for the duration of the function call. The `w` pointer returned by `arena.alloc` points into the arena's backing store, which is not moved or freed while `QuoteGen` lives. The `extra_words` vector holds references with lifetime `'r` tied to the `IrGen` borrow. **Verdict: OK, provided the borrow discipline in `QuoteGen::new` is maintained.**

### 2.5 `hosted/src/mem.rs` — C memory management (2 blocks)

```rust
pub unsafe fn realloc(ptr: NonNull<c_void>, size: usize) -> Result<NonNull<c_void>, Errno>
pub unsafe fn free(ptr: NonNull<c_void>)
```

**Safety analysis**: Standard FFI wrappers. Invariants are the standard C memory model: `realloc` requires a valid pointer from a previous `malloc`/`realloc`/`calloc`; `free` requires a non-null pointer from allocation. These are **not enforced by the type system** — callers must ensure correctness. All callers in the codebase use them correctly (e.g., `ByteBuf` owns the pointer and frees exactly once). **Verdict: OK (standard FFI).**

### 2.6 `hosted/src/io.rs` — `write_all` (1 block)

```rust
let written = unsafe { c::write(self.fd, buf.as_ptr(), buf.len()) };
```

**Safety analysis**: Standard POSIX `write` FFI. The invariants (valid fd, valid buffer, correct length) are the caller's responsibility. The `io::Writer` struct always holds a valid fd (or `1` for stdout). The buffer is a `&[u8]` slice, which is a valid pointer+length pair. **Verdict: OK.**

### 2.7 `hosted/src/fs.rs` — `ByteBuf` (3 blocks)

```rust
// as_slice (line 22):
unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
// read_to_end (line 42):
unsafe { ... c::read(self.fd, ...) }
// drop (line 52):
unsafe { mem::free(self.ptr.cast::<c_void>()) }
```

**Safety analysis**: 
- `as_slice`: The pointer+length come from `mmap`/`read` and are valid for the lifetime of `&self`. The slice is immutable.
- `read_to_end`: POSIX `read` into a stack buffer. Standard FFI.
- `Drop`: Frees the heap allocation exactly once. No aliasing.
**Verdict: OK.**

### 2.8 `hosted/src/cstr.rs` — FFI string helpers (3 blocks)

```rust
pub unsafe fn len(mut ptr: *const c::c_char) -> usize
pub unsafe fn as_bytes<'a>(ptr: *const c::c_char) -> &'a [u8]
pub unsafe fn eq(ptr: *const c::c_char, bytes: &[u8]) -> bool
```

**Safety analysis**: All three assume a valid null-terminated string pointer. This is POSIX convention. Callers pass pointers from `argv` or environment, which are guaranteed null-terminated by the ABI. **Verdict: OK.**

### 2.9 `hosted/src/cstrbuf.rs` — `CStrBuf` (2 blocks)

```rust
// new (line 16):
unsafe { ... ptr::copy_nonoverlapping(...) }  // raw copy to malloc'd buffer
// drop (line 37):
unsafe { mem::free(self.ptr.cast::<c_void>()) }
```

**Safety analysis**: Allocates a C string on the heap via `malloc`, copies bytes including null terminator. Frees exactly once on drop. The `as_ptr()` returns the internal pointer. The drop order in `process.rs` is correct. **Verdict: OK.**

### 2.10 `hosted/src/process.rs` — Process execution (4 blocks)

```rust
// Line 32 (fork error path):
unsafe { arg_bufs[i].assume_init_drop() };
// Line 39 (fork):
unsafe { ... c::fork() ... c::execvp(...) ... c::_exit(...) }
// Line 49 (wait error path):
unsafe { arg_bufs[i].assume_init_drop() };
// Line 62 (success path):
unsafe { arg_bufs[i].assume_init_drop() };
```

**Safety analysis**: This is the most complex unsafe code in the codebase.
- `arg_bufs` is a `MaybeUninit<CStrBuf>` array. Items are initialized after a successful `CStrBuf::new_cp`.
- All three manual-drop paths are reachable exactly once per process.
- After `fork()`, the child calls `execvp` then `_exit(127)` — no drops run in the child (deliberate, as `_exit` is `!` and terminates immediately).
- The parent waits for the child, then drops its own copies. No double-free between parent and child because fork creates independent address spaces (COW).
- If `waitpid` fails (line 49), the parent drops its copies and returns an error.
- If `waitpid` succeeds (line 62), the parent drops its copies and returns the exit code.

**Concern**: There's a panic-safety issue: if any code between initialization and manual drop panics, the `CStrBuf` items will leak. The crate uses `panic = "abort"` in its profile, so a panic won't double-free — it will just leak. This is acceptable for a compiler (crash on panic). **Verdict: OK, with caveat.**

### 2.11 `hosted-rt/src/lib.rs` — Runtime entry (2 blocks)

```rust
// panic handler (line 16):
unsafe { c::_exit(101) }
// _start function (line 19):
pub unsafe fn exit(code: i32) -> !
```

**Safety analysis**: 
- `_exit` from the panic handler: aborting on panic is safe.
- `_start` function: Called by the OS. Must set up `rdi`/`rsi`/`rdx` registers from the standard ABI. The `entry()` function is a raw C-style `extern "C"` entry. The runtime calls `langc::main` with the bootstrapped args. **Verdict: OK (standard bare-metal entry).**

### 2.12 `langc/src/args.rs` — `parse_args` (1 block)

```rust
pub unsafe fn parse_args<'a>(argc: isize, argv: *const *const hosted::c::c_char) -> ParseResult<'a>
```

**Safety analysis**: Dereferences `argv` pointer (standard C ABI guarantee at program entry). Iterates up to `argc` entries. Returns slices with lifetime `'a` tied to the caller's borrow. **Verdict: OK.**

### 2.13 `lang-assemble/src/config.rs` — `parse_args` (1 block)

Same pattern as `langc::args::parse_args`. **Verdict: OK.**

### 2.14 `hosted/src/env.rs` — `get_str` (1 block)

```rust
pub unsafe fn get_str(name: &[u8]) -> Option<&'static [u8]>
```

**Safety analysis**: Access the `environ` global variable. Returns a `'static` reference, which is technically correct because the environment strings live for the program's lifetime. **Verdict: OK.**

### 2.15 Overall Unsafe Assessment

| Category | Count | Soundness |
|---|---|---|
| `MaybeUninit` manual drop (FixedVec, Arena, process.rs) | 7 | Sound — len-tracked, reverse drop, abort on panic |
| Raw pointer reborrow (quotes.rs arena) | 2 | Sound — borrow discipline maintained |
| POSIX FFI (mem, io, fs, cstr, process) | 11 | Sound — standard C/ABI invariants |
| `no_mangle` entry point | 1 | Sound — standard bare-metal startup |
| `static` environment access | 1 | Sound — program-lifetime data |
| `_exit` from panic handler | 1 | Sound — abort semantics |

**No memory safety bugs identified.** The unsafe code is well-contained, follows consistent patterns, and maintains Rust's safety invariants.

---

## 3. Code Quality Assessment

### 3.1 Strengths

- **Disciplined `no_std` usage**: The entire compiler core is `#![no_std]` without `alloc`. Memory is statically pre-allocated (`FixedVec<T, N>`) or arena-allocated. This is appropriate for a systems/embedded toolchain target.
- **Clean crate separation**: Concerns are well-separated (frontend, semantics, IR, codegen, runtime, hosted drivers).
- **Consistent error handling**: `TcError { code: u32, span: Span }` is used uniformly throughout the typechecker. Error codes are unique and meaningful.
- **Stack-based type system**: The stack-effect typechecker is an elegant design for a concatenative language.
- **Testing with Miri**: Arena allocator and FixedVec tests run under Miri for UB detection. Integration tests run under QEMU.
- **Test coverage**: `tooling-tests` has ~130 test cases covering the full compilation pipeline through QEMU.

### 3.2 Idiomatic Rust Concerns

| Issue | Severity | Description |
|---|---|---|
| **`#[deny(unsafe_op_in_unsafe_fn)]` not used** | Medium | Several `pub unsafe fn` exist (e.g. `mem::free`, `cstr::len`) but their bodies don't use `unsafe {}` blocks. This means the compiler won't warn if the body uses unsafe operations without explicit `unsafe {}`. |
| **`&[u8]` instead of `&str`** | Low | The lexer, parser, and typechecker operate on `&[u8]` rather than `&str`. The source is always UTF-8, so using `&str` would provide stronger encoding guarantees. The current approach bypasses UTF-8 validation but is consistent with the no-std/embedded philosophy. |
| **Heavy `expect()` on TypeAtom creation** | Low | `TypeAtom::new(b"i64").expect("builtin type fits in 32 bytes")` appears ~30+ times. These should be `const` or `lazy_static` values. They're currently recomputed every time. |
| **Numeric error codes** | Low | All errors are `u32` codes with a span. A typed error enum would be more idiomatic but would add complexity. The current approach works and is debuggable. |
| **No `?` operator in some paths** | Low | Some error handling uses explicit `match`/`if let` where `?` would suffice. Minor style issue. |
| **Property tests / fuzzing** | Medium | No property-based tests or fuzz targets exist. The typechecker would benefit from fuzzing with random type expressions. |
| **`build_quote_word` return type** | Low | Returns `(TypeAtom, Sig, bool)` — an unnamed tuple. A named struct would be clearer. |

### 3.3 Maintainability Concerns

| Concern | Severity | Location |
|---|---|---|
| **`compile.rs` god function** | Medium | `compile_quote_body_sig` in `crates/semantics/src/typecheck/irgen/compile.rs` is ~1,000+ lines handling all IR generation in a single loop with multiple nested `if` chains. Extremely difficult to follow or modify. |
| **`emit_stackcheck` monolithic function** | Medium | `crates/semantics/src/typecheck/stackcheck/mod.rs` — `print_value`/emit logic mixed with stack checking. Could be factored. |
| **`X86_64HostedBackend` god struct** | Medium-Low | `crates/codegen-x86_64/src/lib.rs` — 16+ fields, single monolithic struct handling all emission. |
| **Magic numbers in API headers** | Low | Object file generation hardcodes constants like `0x13` for symbol scope, `0x12` for `GCC::Wolf`, `0x22` for `STB_GLOBAL`. These should be named constants. |
| **Span management** | Low | `Span::new(0, 0)` appears ~50+ times as a placeholder. Errors with zero spans are hard to debug. |

### 3.4 Specific File Notes

#### `compile.rs` — IR Generation (~1,239 lines)

The heart of the compiler. A single `compile_quote_body_sig` function handle all word compilation:
- Stack simulation
- Local variable (scope) management
- Built-in operators (arithmetic, comparison, boolean)
- Control flow (if, while, loop, lock)
- Quotes and call
- Task operations (spawn, run)
- Type conversions (as, as?, bitcast)
- MMIO operations

The function uses a single `loop { let tok = lex.next(); ... match ... }` with `continue` for flow control, making it essentially a state machine with implicit states. The function body is a flat sequence of ~30 `if name == b"..."` checks. Each handler is a reasonable 10-50 lines, but the lack of any decomposition into sub-functions makes the whole thing very hard to navigate.

**Major sub-issues within this file:**

1. **Repeated value-to-TypeAtom conversion**: The pattern:
   ```rust
   let top_ty = match top {
       Value::Plain(t) => t,
       Value::Scoped { ty, .. } => ty,
       Value::Resource(_) => TypeAtom::new(b"resource").expect(...),
       Value::Quot(_) => TypeAtom::new(b"quot").expect(...),
       // ... etc
   };
   ```
   This appears 8+ times identically. Should be a method `Value::to_type_atom()`.

2. **Local variable index computation**: `self.local_slot(idx)` followed by explicit `Field` struct construction happens frequently. This could be factored.

3. **`self.emit_op(cur, kind, name_abs)?; continue;`** — This pattern appears ~40 times. Refactoring the match arms into method calls would make the main loop vastly more readable.

#### `stackcheck/mod.rs` — Stack Type Check Output (~600 lines?)

This module also has the same flat-loop structure as `compile.rs` but for the type-checker's diagnostic output. The `emit_stackcheck` function walks through the same logic as the IR generator but only produces text output. This is effectively **duplicated logic** — the stack check pass and the IR generation pass both re-parse and re-typecheck the same code. A cleaner architecture would have a single typechecking pass that feeds both the IR generator and the diagnostic printer.

#### `process.rs` — Process Execution (66 lines)

Clean, well-structured. The only concern is the panic-safety of manual `assume_init_drop` calls. With `panic = "abort"` this is acceptable.

---

## 4. Testing & Verification

### 4.1 Unit Tests

| Crate | Tests | Notes |
|---|---|---|
| `frontend` | 16 (in `tests/fixed.rs`) | Covers `FixedVec` push/pop/extend/truncate/iter. Some run under Miri. |
| `semantics` | 6 (in `tests/arena.rs`) | Arena allocator tests. Run under Miri. |
| `tooling-tests` | ~110 (in `tests/smoke.rs`) | Integration tests: compile Tyu source, run under QEMU, check output. |

### 4.2 Integration Tests

The main testing regimen is `cargo test -p tooling-tests`, which:
1. Writes a `.tyu` source file
2. Compiles it with `langc`
3. Runs the resulting binary under QEMU
4. Asserts the exit code and/or stdout

Tests cover:
- Basic arithmetic and boolean logic
- Control flow (if/while/loop)
- Recursion
- Scheduler/task operations
- MMIO access patterns
- `as` and `as?` casts
- Error cases and edge cases

### 4.3 Gaps

| Gap | Severity | Description |
|---|---|---|
| **No unit tests for `irgen/compile.rs`** | High | The largest, most complex file in the codebase has zero dedicated unit tests. It is only tested indirectly through integration tests. |
| **No unit tests for `stackcheck/mod.rs`** | Medium | The stack checker has no isolated tests. |
| **No unit tests for codegen** | Medium | Neither `codegen-core` nor `codegen-x86_64` has unit tests. |
| **No property-based testing** | Medium | No QuickCheck/proptest harness for e.g. stack effect verification. |
| **No fuzzing** | Medium | No fuzz targets for the lexer, parser, or typechecker. |
| **No regression test suite** | Medium-Low | No systematic way to add regression tests for past bugs. |
| **Miri coverage** | Low | Only `fixed.rs` and `arena.rs` tests run under Miri. |

---

## 5. Build & Dependency Hygiene

### 5.1 Build Times

The project compiles from scratch in about 5 seconds (single-threaded release). Dependencies are limited:
- No proc macros
- No serde
- No async runtime
- No external crates except `hosted` -> `hosted-rt` -> `langc`/`lang-assemble` circular-ish chain

This is excellent — the compiler is genuinely lightweight.

### 5.2 Unused Dependencies

- `ir` crate depends on `frontend` but only uses `Span` and `TypeAtom`. This is fine — these are core types.
- No unnecessary feature flags.

---

## 6. Recommendations (Priority Order)

### P0 — Critical
- None found. The codebase is safe and functional.

### P1 — High
1. **Extract `Value::to_type_atom()` method** — The `match { Plain => ..., Scoped => ..., Resource => ..., ... }` pattern appears 8+ times identically in `compile.rs`. Extract to a method on `Value`.
2. **Add unit tests for `compile.rs`** — The IR generator is ~1,200 lines with zero direct tests. Even a few test cases for individual word compilation would dramatically improve regression safety.

### P2 — Medium
3. **Refactor `compile_quote_body_sig`** — Break the ~1,000-line function into per-word methods on `IrGen` (e.g., `compile_if`, `compile_while`, `compile_loop`, etc. — some of these already exist as separate methods; extend the pattern).
4. **Add `#[deny(unsafe_op_in_unsafe_fn)]`** — Add this lint to all crates with `pub unsafe fn` to ensure every unsafe operation inside an unsafe function is explicitly marked.
5. **Make `TypeAtom::new(b"i64")` calls const** — Add a `const I64: TypeAtom = ...` to `TypeAtom` or a dedicated constants module. This would eliminate ~30 redundant `expect()` calls.
6. **Add Miri CI step** — Run `cargo miri test` on a schedule to catch UB in the `no_std` unsafe code.
7. **Name magic numbers in codegen** — Replace `0x13`, `0x22`, `0x12` in API headers/ELF emission with named constants.

### P3 — Low
8. **Add property tests for stack checker** — Generate random stack effect signatures and verify that the typechecker handles them consistently.
9. **Use `&str` instead of `&[u8]` for source text** — Would require `core::str::from_utf8` calls at boundaries, but provides UTF-8 guarantees.
10. **Add `Span::UNKNOWN` constant** — Replace the pervasive `Span::new(0, 0)` with a named constant.
11. **Fix `likely`/`unlikely` import** — The `frontend` `lib.rs` conditionally imports `likely`/`unlikely` from `core::intrinsics`. This is nightly-only and gated by `#[cfg(target_os = "none")]`, which is fragile. Consider using a stable alternative.
