# Implementation Status (Linux hosted focus)

This table is a living inventory mapping the specs to what is implemented in the repository today (especially for the Linux hosted toolchain).

Legend:
- **Frontend** = lex/parse/AST
- **Semantics** = typechecking, static rules, IR building
- **Backend** = code generation (Linux hosted)
- **Runtime** = platform/sysroot + linked/runtime components
- **Tests** = where validated

## Compiler driver / artifacts

| Feature | Spec anchor | Status | Notes / code | Tests |
|---|---|---|---|---|
| `langc --emit=ast` | Compiler spec §2.1 | Implemented | `crates/langc/src/main.rs` | `crates/tooling-tests/tests/smoke.rs` (`langc_emit_ast_simple_module`) |
| `langc --emit=ir` | Compiler spec §2.1 | Implemented | `crates/semantics/src/typecheck.rs` (IR emission) | `crates/tooling-tests/tests/smoke.rs` |
| `langc --emit=asm` | Compiler spec §2.1 | Implemented (hosted-only) | Emits **FASM ELF64 executable** (`crates/langc/src/main.rs`) | `crates/tooling-tests/tests/smoke.rs` (`milestone7_*`) |
| `langc --emit=obj` | Compiler spec §2.1 | Missing | No object emission | N/A |
| `langc --emit=bc` | Compiler spec §2.1 | Missing | No bytecode backend | N/A |
| `--target=<triple>` | Compiler spec §2.1 / Runtime spec §3 | Stub | Flag exists but not implemented | N/A |
| `--out-dir=<path>` | Compiler spec §2.1 | Stub | Flag exists but not implemented | N/A |

## Modules / interface model

| Feature | Spec anchor | Status | Notes / code | Tests |
|---|---|---|---|---|
| `module/import/export/end` parsing | Language ref §3 | Implemented | `crates/frontend/src/lex.rs`, `crates/frontend/src/parse.rs` | `crates/tooling-tests/tests/smoke.rs` |
| `.def`/`.mod` conformance | Compiler spec §3.2 | Implemented | `crates/langc/src/main.rs` (`check_iface`) | `crates/tooling-tests/tests/smoke.rs` (`iface_*`) |
| Import resolution via `-I` / `--sysroot` | Compiler spec §3.3 | Implemented (simple) | `crates/langc/src/main.rs` (`try_load_module_file`) | `crates/tooling-tests/tests/smoke.rs` |

## Core control intrinsics and quotations

| Feature | Spec anchor | Status | Notes / code | Tests |
|---|---|---|---|---|
| `if/while/loop/lock` | Compiler spec §7 | Implemented (typecheck+IR) | `crates/semantics/src/typecheck.rs` | `crates/tooling-tests/tests/smoke.rs` (`milestone7_if_while_locals_smoke`) |
| Effect set token `!{suspend}` | Language ref §7.3 / Compiler spec §6 | Implemented (minimal) | lexer parses `!{...}` as `EffectSet` | `crates/tooling-tests/tests/smoke.rs` (suspend-related cases) |
| Escaping typed quotations + `call` | Compiler spec §6 | Missing/incomplete | No general first-class quotation calling model | N/A |

## Contracts + subtypes

| Feature | Spec anchor | Status | Notes / code | Tests |
|---|---|---|---|---|
| `requires` / `ensures` syntax | Language ref §10.2 | Implemented | `crates/frontend/src/parse.rs` | `crates/tooling-tests/tests/smoke.rs` |
| Contract trap insertion | Compiler spec §5.3 | Implemented (IR) | `crates/semantics/src/typecheck.rs` (`emit_prologue/epilogue`) | Some coverage in `smoke.rs` |
| Subtype decl + range checks | Language ref §10.1 / Compiler spec §5.2 | Partially implemented | Range trap insertion exists; boundary policy needs completion | `crates/tooling-tests/tests/smoke.rs` |
| Runtime trap codes | Runtime spec §5.3 | Implemented (hosted) | `sysroot/Core.mod` + `__lang_trap` in emitted asm | `milestone7_*` tests indirectly |
| `__lang_trap_loc` | Runtime spec §4.1 / §10.3 | Missing | Not emitted/linked | N/A |

## Places, borrows, MMIO

| Feature | Spec anchor | Status | Notes / code | Tests |
|---|---|---|---|---|
| Place resolution (`a.b.c`) | Compiler spec §8.1 | Implemented | `crates/semantics/src/typecheck.rs` (`resolve_place_pointee_ty`) | `smoke.rs` |
| `&place` / `&!place` | Compiler spec §8.2 | Implemented (typecheck+IR) | `crates/semantics/src/typecheck.rs` (borrow tokens) | `smoke.rs` |
| Scoped borrows `&[ ]` / `&![ ]` | Compiler spec §8.3–§8.5 | Partial | Enforces “no scoped live”; does not implement Array→Slice or Region handles yet | `smoke.rs` has suspend rule checks |
| `register-map` parse + legality checks | Compiler spec §9 | Implemented (static) | `crates/frontend/src/parse.rs`, `crates/semantics/src/typecheck.rs` | `smoke.rs` (MMIO legality) |
| MMIO codegen (volatile loads/stores/fields) | Compiler spec §9 | Missing | IR ops exist but backend returns errors | N/A |

## Concurrency surface (hosted)

| Feature | Spec anchor | Status | Notes / code | Tests |
|---|---|---|---|---|
| `platform.task.yield` surface | Runtime spec §6.7 / §8.2 | Stub | Declared `{suspend}` but backend treats as no-op | `smoke.rs` (suspend checks) |
| `platform.channel.*` surface | Runtime spec §6.8 / §8.4 | Implemented as backend intrinsic | Fixed ring buffer, `Chan(i64)` only | `smoke.rs` (`platform.channel.make/send/recv`) |
| `iso` / move-only | Language ref §13.3 / Compiler spec §5.1 | Missing | `iso` keyword not tokenized; no move-only rules | N/A |

## How to validate hosted end-to-end today

- Build tools: `cargo build`
- Run integration suite: `cargo test -p tooling-tests`
- Ensure `fasm` is installed and on PATH (required for tests that assemble and run emitted asm).

