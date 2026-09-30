# Appendix B — Error and Trap Registry

> Every code in this book's labs, with the fixture that earned it. The
> checker's error codes are falsifiable claims: each one exists because a
> violation exists. Bands: `10xx–70xx` compiler front end, `32xx–37xx`
> typecheck, `36xx` MMIO, `50xx` effects/contexts, `51xx` stack bounds,
| `52xx` loader.

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
| E3312 | contract modified its inputs (identity — a `swap`/slot-replacement now fires, slice P6) | (registry; shape rarely reachable) |
| E3313 | contract predicate is impure (an effect, store, or spawn inside `needs`/`ensures`) | (slice P6 — a predicate is a question) |
| E3314 | contract predicate too large (peak > 64 stack slots) | (slice P6) |
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
> or region exhaustion, and the docs say so in exactly those words (Q14).
> The static-verification plan's obligation schema treats them as defined
> runtime behavior, not proof obligations.

## Verification pipeline (chapter 14)

| Code | Meaning | Verified |
|------|---------|----------|
| E6416 | verify toolchain failed (lake/harvest) — fail-closed, all open | yes (stale-scaffold build) |
| E6418 | generated-statement digest mismatch (Gen edited) | tamper suite |
| E6419 | axiom audit failed / assumption closure malformed | port gate |
| E6420 | harvest: expected statement missing from the kernel environment | tamper suite |
| E6421 | verdict stale: `statement_hash`/target mismatch at consumption | (consumption path) |
| E6510 | `proven` policy vs unmodeled bundle or unproven callee | (deploy gate) |

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

## Verification artifact band (64xx)

Compile-time codes of the verification subsystem
(`devdocs/plans/static-verification.md`, slices P2+; owner doc
`devdocs/plans/design-doc/verification-obligations.md`). These are **not**
runtime trap codes; they live in the claim table so artifacts and report
tooling share one registry.

| Code | Meaning |
|------|---------|
| E6400 | `.obl.json` artifact schema version is not `tyu.obl/v2` (fail-closed read) |
| E6401 | `.obl.json` artifact is malformed / failed to encode (fail-closed: checks stay) |
| E6402 | verdicts file (`--verdicts`) malformed, wrong schema/semantics, or oversized — the compile aborts before codegen (fail-closed: a bad file can only cause more checking) |
| E6403 | descriptor `[verification]` section invalid (grants non-positive, or a stale `data_stack_slots` key) |
| E6410 | `--verify-policy=no-open` / `no-open-no-assumptions` / `proven` failure: the build lists every open (and, with `-no-assumptions`/`proven`, assumed) obligation with its location |
| E6413 | stale contract interface: a `.def`/decl names a predicate the callee's `.obl.json` artifact does not document (slice P6 — fail-loud, never a silent open); also the reject code for an unsupported `.def` clause such as `bound` |
| E6415 | image verdicts record invalid (`.tyu-verify/image-verdicts.json`, slice P7 — the guard-elision decision evidence is malformed/version-mismatched; fail-loud, never a silent different guard state) |
| E6416 | developer-proof pipeline failure class (PLAN-VERIFY-3 P6/P7): missing/mismatched Lean toolchain, `lake` build failure, harvest crash or non-zero exit, lockfile contention timeout — fail-closed: verdicts not written, every obligation open |
| E6417 | closed-registry violation in a verdicts file (§Q6): unknown `method` / `proof.kind` / `trust` (PLAN-VERIFY-3 §6.3) |
| E6418 | Gen-digest mismatch (PLAN-VERIFY-3 P6.1/P7.1): the generated statement surface or vendored semantics don't match the artifacts the build computes — the proof environment is tampered/stale, pre-`lake` gate |
| E6419 | axiom-audit failure in the harvest (PLAN-VERIFY-3 §Q11): a theorem's proof relies on a non-benign axiom (`sorryAx`, `Lean.ofReduceBool`, or any axiom outside `{propext, Quot.sound, Classical.choice}`); also malformed assumption-closure |
| E6420 | harvest missing-statement (PLAN-VERIFY-3 FR-4): the Gen metadata names a `stmt_*` def absent from the kernel environment (E6420 path) |
| E6421 | statement-stale (PLAN-VERIFY-3 FR-5): a consumed verdict's `statement_hash` (or `(target, model_semantics)` identity) no longer binds the current build — fail-closed to open, check retained, counted |

### Report-field glossary (slices P3–P8)

`verify-report.json` (`tyu.verify-report/v2`, written by `tyu build` beside
the image; owner doc `verification-obligations.md` §6.5). Every field is
enforced by a gate, not by convention:

- `modules[]` — per-module class accounting (`subtype-range` /
  `contract-pre` / `contract-post` / `stack-budget` / `mmio-bounds`: total,
  discharged, assumed, open).
- `contexts.stack` — the per-context budget verdicts: `main` (high, top,
  `N_main` *derived* from the runtime binary's DS geometry, verdict) and
  `isr` (`N_isr` from the `[verification]` grant); `guards` =
  `"retained"` | `"elided"` (slice P7 — a truthful label of the object).
- `open[]` — obligations whose verdict is not closed, with the interval
  engine's reason when one was recorded (drives E6410).
- `provably_failing[]` — interval-proven *always* out of range; the check is
  retained and the report names why (never a discharge).
- `assumed[]` — human decisions, recorded with their justification.
- `retained[]` — checks kept for a policy reason (e.g. dynamic exports under
  `module-loading`, slice P6/FR-21).
- `assumptions_trusted[]` — `runtime` facts (derived `N_main`) and
  `descriptor` facts (isr grant, aperture sizes) a discharge relied on —
  counted as *trusted*, never folded into "proven".
- `emitted_checks` — the truth block (`subtype_range`, `contract`,
  `mmio_bounds`, `data_stack_guards`) MUST agree with the object code
  (FR-16; the corpus gate + slice tests enforce the bijection).
- `stale_verdicts`, `verdict_sources` — input verdicts that matched nothing,
  and the file-vs-in-tree discharge split.
- `trust` / `methods` / `surfaces` (per-module, PLAN-VERIFY-3 §Q6/§7.3) — the
  v2 trust-class split (`proof` / `checked` / `assumed` / `open`), the
  closed-method split, and the certificate surface split (`source` / `ir`).
- `tcb[]` — the shipped trust-computing-base boundary (§6.8): each
  trust-bearing component named with its status (`assumed`, `reviewed+vectors`,
  `drift-locked`, `theorem`, `structural`, `structural+modeled`, `empirical`).
- `proof` — the P6.2 section: the proof pipeline's status (`none` /
  `lean-skipped` / `lean`), `harvest` (`not-built` / `ok`), `gen_digest`
  (`verified` / …), and the per-module statement accounting (`rendered`,
  `proven`, `unproven`).
- `policy` — the enforced verify policy (`open-ok` / `no-open` /
  `no-open-no-assumptions` / `proven`; `off` under `--verify=off`).

P6 extends the band: E6413 (stale interface / `.def`↔`.obl` hash
disagreement). P7 (PLAN-VERIFY-3 P6/P7) adds: E6416 (proof-pipeline failure
class), E6417 (closed-registry violation), E6418 (Gen digest), E6419 (axiom
audit), E6420 (harvest missing statement), E6421 (statement-stale at
consumption) — each allocated in §6.9 of the developer-proof-pipeline plan
with its owning phase.

## Manifest / certification band (65xx)

The in-module `verify_manifest` record (P11.1/P11.2) and the certification
package (P11.3). Loader codes are on-device admission (the loader never
interprets proofs — it validates integrity and applies the platform policy);
the deploy/package codes are host-side.

| Code | Meaning |
|------|---------|
| E6500 | `verify_manifest` record malformed (position/order/closed-set violation) — the loader rejects the module before any allocation |
| E6501 | `verify_manifest` verdict-digest mismatch: the digest over the carried obligations region does not recompute |
| E6502 | platform `verify_policy` reject: the module's declared policy does not satisfy the platform's requirement (`RequireNoOpen`/`RequireProven`) |
| E6503 | certification-package pairing (E_CERT_PAIRING): a B1 module digest or B2 manifest digest mismatch — the package does not bind the shipped bytes (FR-21 fail-closed at deploy + `tyu cert verify`) |
| E6504 | certification index malformed (`tyu.cert/v1` reader: schema mismatch, size cap, canonical-order violation) |
| E6510 | proven pairing against an unproven callee or an `unmodeled` bundle, at build AND deploy (E_MODEL_UNMODELED, §Q7 rule 3 / §Q15 / FR-8) |
