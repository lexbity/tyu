# Assembling in Tyu — draft v3

> *Safety is not the brake — it's the engine: the rigor is what licenses the
> daring parts.*

## Table of contents (agreed structure)

**Part I — Philosophy and the medium**
1. The Rigor Is the Engine *(drafted)*
2. First Words *(drafted)*

**Part II — The disciplines**
3. Types: Making Illegal Values Unrepresentable *(drafted)*
4. Contracts: Promises the Machine Reads *(drafted)*
5. Effects and Capabilities: The Two-Sided Law *(drafted)*
6. The Stack Has a Shape *(drafted)*
7. Places, Borrowing, Ownership *(drafted)*
8. Touching Hardware: MMIO *(drafted)*

**Part III — Design and style**
9. Factoring: Style Becomes Proof *(drafted)*
10. Decomposition: Information Hiding, Enforced *(drafted)*
11. Shared State and the Cross-Context Rule: Interrupts *(drafted)*
12. Tasks and Channels: Concurrency the Platform Provides *(drafted)*

**Part IV — The daring parts**
13. Field Updates on Untrusted Media *(drafted)*

**Part V — Proving the program**
14. Proving the Program: The Developer-Proof Pipeline *(drafted)*
15. The Machine as Witness: Emulation in Verification *(drafted)*
16. Epilogue: Tyu's Effect on Thinking *(drafted; renumbered from 14 — the proof chapters displaced it)*

**Appendices**
A. Language reference (duplicated; chapters teach, the appendix defines) — includes table A.1, the catalog of built-in words, cross-referenced from ch. 2
B. Error-code & trap registry (cross-linked to lab fixtures)
C. Toolchain & QEMU setup (hosted-first; metal interludes)
D. One-page style summary
E. Design dialogues *(separate session; interview format)*

## Chapter skeleton (per chapter)

Style rules for all drafts: assume nothing about what the reader knows and
never remark on what they may know; address the reader as "you" sparingly
(prefer the imperative and declarative sentences); never call the text or the
language "honest" — the transcripts are real, and that is for the reader to
judge.

1. **The Practice** — the one checkable discipline of the chapter.
2. Prose sections building the discipline from experience to principle.
3. **Labs** — green / red / engine, full source inline, with real transcripts.
   A green lab may be marked **(build-the-vocabulary)** when it constructs a
   missing stdlib word in Tyu.
4. **Post-mortem** — *optional*, aimed at readers with C/embedded scars: the
   project-killing anti-pattern, first as the C disaster it is, then as the
   rejected compile or clean trap. Newer readers may skip.
5. **From the workbench** — short author aside: why the rule is the rule.
6. Exercises (predict-then-run preferred over filler).

## Toolchain quirks discovered while drafting (for the author)

- **RESOLVED — hosted platform flow**: every pack now declares
  `[platform] kind = "hosted" | "metal"`, and the builder assembles the
  declared `[metal] startup` unit per pack. `tyu build` /
  `tyu run --platform=linux-x86_64-hosted` work (the hosted startup unit
  owns `_start`); `--mode=dynamic` on a hosted runtime is refused early
  with an intentional gate (dynamic images are metal-firmware images — the
  on-device loader and `.modpack` only exist in the metal flow). Hosted
  dynamic module loading remains the host-side (Rust) loader used by the
  tooling suites. The ISR budget-masking and `@interrupt`-on-hosted items
  below are unaffected.

- **ISR budget error masked in the production path**: a handler whose peak
  exceeds its binding-site budget reports `E5030` via `langc --emit=ir`,
  but through `tyu build` the same program surfaces as `E7001` (missing
  word: main) — the typecheck abort masks the real verdict. Red lab 11.5
  uses the inspection path; the masking is worth a toolchain fix.
- **Hosted images with `@interrupt` bindings are not runnable** (they
  build, then crash at startup) — bindings are metal; green lab 11.1 is
  build-only by design.
- **RESOLVED — ARM ISR runtime fixture**: `isr_lock_atomicity.mod`
  (lm3s6965evb SysTick) now runs in the `tyu test` QEMU pipeline. The old
  "interrupt doesn't fire in time" comment was a misdiagnosis: the handler
  didn't stop the timer, so the free-running tick re-latched its pending
  bit and the NVIC tail-chained extra entries (nondeterministically many).
  The handler now stops SysTick on entry and guards against the one
  deterministic tail-chained re-entry (see the fixture's comment).
- **RESOLVED — hosted `tyu run --platform=linux-x86_64-hosted`**: the
  builder now honors the pack's declared `[metal] startup` unit instead of
  assembling a fixed `runtime` stem. Hosted MMIO over the descriptor's
  simulated devices runs through the pack path (chapter 8 may use it; the
  QEMU track remains the metal reference).
- **`tyu test` fixture naming can affect QEMU linking**: a manifest entry
  named `strategies` failed to link where `mmio_strategies_x86` passed
  (identical fixture). Worked around; worth investigating.
- **E3312 (`ContractModifiedInputs`) is unreachable with uniform
  parameter types** — a two-typed-parameter predicate shape may be needed
  to ever exercise it. However, the **obligations pass** (pass-1 of
  `--verify-tool=lean`) fires E3312 on contract predicates that compute
  through helper word calls (origin tracking) — the arithmetic-dialect
  constraint of ch. 14. The same module compiles clean without the
  pipeline: stricter pass, by design.
- **Tamper scripts use relative paths**: `tamper/run-mutate-gen.sh
  harvest-fixture` path-duplicates and greps the wrong file — invoke with
  an absolute fixture path (`"$PWD/harvest-fixture"`).
- **`tyu_auto` on bare system lean** can fail to close goals it closes
  under `ci/port.sh`'s pinned-toolchain flow (simp-set differences); the
  candidate e2e and the port gate are the supported attribution paths.
- **The E5030 corpus shape**: the e5030 fixture's 4th dup line has *three*
  dups (peak 34, net 0) — hand-retyped versions with four E3220 first.
