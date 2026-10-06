# Appendix B — Error and Trap Registry

> Every code in this book's labs, with the fixture that earned it. The
> checker's error codes are falsifiable claims: each one exists because a
> violation exists. Bands: `10xx–70xx` compiler front end, `32xx–37xx`
> typecheck, `36xx` MMIO, `50xx` effects/contexts, `51xx` stack bounds,
> `52xx` loader.

## Compile-time rejections verified in this book

| Code | Meaning | Lab |
|------|---------|-----|
| E1018 | `main` must return exactly one value (the exit code) | 1.4 |
| E2195 | legacy `requires [ … ]` predicate — use `needs [ … ]` | (migration note, ch. 4) |
| E2201 | imported interface not found for this platform | 12.6 |
| E2203 | imported symbol not exported by the interface | 10.4 |
| E2218 | interface/implementation word signature mismatch | 10.3 |
| E2219 | interface/implementation word effect mismatch | 10.5 |
| E3202 | stack underflow | 2.5 |
| E3210 | word not found | 1.6 |
| E3211 | signature stack underflow at a call | (trace idiom, ch. 2) |
| E3212 | call-site type mismatch | 3.3 |
| E3220 | declared stack effect vs computed body | 2.6, 6.2, 9.3 |
| E3242 | `if` cannot pop a condition | (ch. 2 notes) |
| E3243 | `if` condition is not a bool | 2.7 variant |
| E3246 | `if` branch depth mismatch | 2.7 |
| E3251 | `while` cannot pop its condition | (ch. 2 notes) |
| E3253 | `while` condition is not a quotation | (ch. 2 notes) |
| E3257 | `while` body is not net-zero | 6.3 |
| E3304 | `bitcast` size mismatch | 3.6 |
| E3305 | raw pointer cast | 3.7 |
| E3310 | contract verdict count wrong (one bool on top) | 4.4, 4.6 |
| E3311 | contract verdict is not a bool | 4.5 |
| E3312 | contract modified its inputs (identity) | (registry; shape rarely reachable) |
| E3313 | contract predicate is impure (an effect, store, or spawn inside `needs`/`ensures`) | (registry — a predicate is a question) |
| E3314 | contract predicate too large (peak > 64 stack slots) | (registry) |
| E3608 | MMIO bitfield is not addressable | 8.6 |
| E3611 | MMIO register misaligned | 8.5 |
| E3640 | MMIO without a platform descriptor | 8.7 |
| E3644 | MMIO board instance not found | 8.3 |
| E3647 | MMIO register-map rows disagree with the descriptor | 8.4 |
| E3760 | `call`/`spawn` of an unannotated quotation | 12.4 |
| E5001 | suspend where forbidden (main, lock, ISR) | 5.7, 11.3 |
| E5002 | nested lock | 11.2 |
| E5004 | capability missing (resource access outside its lock) | 5.6 |
| E5005 | undeclared effect | 5.5 |
| E5010 | `dup` of an `iso` value | 7.4 |
| E5011 | implicit `drop` of an `iso` value | 7.5 |
| E5012 | use after move | 7.6 |
| E5020 | borrow escaped its block | 7.7 |
| E5021 | conflicting live borrows of one root | 7.8 |
| E5022 | `dup` of a live mutable borrow | (ch. 7 prose, verified) |
| E5030 | ISR stack exceeds its budget | 11.5 |
| E5031 | interrupt-reachable resource accessed unlocked | 11.4 |
| E5040 | unbounded (self-recursive) handler | 6.4 |

## Runtime trap codes

Hosted exit status equals the trap code.

| Code | Name | Fires when | Met in |
|------|------|------------|--------|
| 10 | `STACK_OVERFLOW` | data-stack push past the limit | (defense in depth — ch. 6) |
| 20 | `CONTRACT_FAIL` | a `needs`/`ensures` promise broke | 1.3, 4.3, 5 |
| 21 | `SUBTYPE_FAIL` | a value left its declared range | 3.4 |
| 22 | `ASSERT_FAIL` | an assertion failed | (registry) |
| 23 | `UNREACHABLE` | a state declared impossible was reached | (registry) |
| 24 | `TASK_QUEUE_OVERFLOW` | **environmental**: the scheduler's task queues filled (a runtime resource, not a language-invariant violation) | (registry, ch. 12) |
| 25 | `DEADLOCK` | **environmental**: all tasks blocked and nothing is runnable (a runtime scheduling fact) | (registry, ch. 12) |
| 26 | `REGION_EXHAUSTED` | **environmental**: an allocation region ran out (a runtime resource) | (registry) |

> **Environmental failure semantics.** Traps 24/25/26 are *environmental*
> failure classes, not language-invariant violations: a program proven under
> the verification discipline can still die by task-queue overflow, deadlock,
> or region exhaustion. They are defined runtime behavior, not proof
> obligations.

## Verification pipeline (chapters 3, 4, and 14)

| Code | Meaning | Verified |
|------|---------|----------|
| E6416 | verify toolchain failed (lake/harvest) — fail-closed, all open | yes (stale-scaffold build) |
| E6418 | generated-statement digest mismatch (Gen edited) | tamper suite |
| E6419 | axiom audit failed / assumption closure malformed | port gate |
| E6420 | harvest: expected statement missing from the kernel environment | tamper suite |
| E6421 | verdict stale: `statement_hash`/target mismatch at consumption | (consumption path) |
| E6510 | `proven` policy vs unmodeled bundle or unproven callee | (deploy gate) |
| E6410 | `--verify-policy=no-open` / `no-open-no-assumptions` / `proven` failure: the build lists every open (and assumed) obligation with its location | ch. 3, ch. 4 (policy) |
| E6413 | stale contract interface: a `.def` names a predicate the callee's artifact does not document | ch. 4 (obligations) |

## Loader verdicts (chapter 13)

| Code | Refusal |
|------|---------|
| E5200 | ABI hash mismatch |
| E5201 | bad container (length/offset validation) |
| E5202 | signature invalid |
| E5203 | module declares an interrupt handler (static-ISR rule) |
| E5204 | unsupported relocation |
| E5205 | unresolved import |
| E5219–E5224 | platform/aperture/modinfo version family |

Suite: `cargo test -p execution-tests --test dynamic_negative` executes
ten of these as attempted crimes (Lab 13.4).

