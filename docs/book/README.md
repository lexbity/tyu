# Assembling in Tyu — draft v3

> *Safety is not the brake — it's the engine: the rigor is what licenses the
> daring parts.*

Draft complete: 16 chapters + appendices A–D, every lab executed against the toolchain (79 language fixtures + the proof-pipeline and port-gate transcripts). Appendix E reserved; F planned. Working draft of the book (working title v3: **Assembling in Tyu**), written
in the open inside the `tyu_lang` repository. Audience: students — the book
assumes nothing except the installed toolchain and a computer. Every lab runs
on the hosted Linux target (`x86_64-unknown-linux-gnu`); no boards, no
simulators, unless a chapter's concept demands metal, and then it is an
explicitly marked interlude.

## Reader profile and locked decisions

- **Reader:** a student with **no prior programming experience** assumed. Every
  concept is defined at first use. Post-mortems assume C/embedded scars and are
  **optional sidebars** — marked so newer readers can skip them.
- **Platforms:** hosted-first everywhere (`x86_64-unknown-linux-gnu`).
  QEMU installation is *recommended* (embedded development needs it) and up to
  two marked metal interludes may require it.
- **Missing vocabulary is a lab, not a gap:** where the stdlib lacks a word
  (no `over`, no number-to-string, …), the book *builds it in Tyu* as a
  green "build-the-vocabulary" lab. The stdlib is Tyu source; that is the
  point.
- **Appendix C** documents the current repo setup procedure (temporary, until
  a script/rustup-style installer exists).
- Prior art: `devdocs/code-book-v2-draft/` (PASS/COUNTER-EXAMPLE suites,
  verified at commit `109a89e`) — mined for idioms; everything re-verified
  on the current tree before use in v3. Notable finds: the `if`-based
  two-sided contract idiom (satisfies the E3310 predicate rule), hosted MMIO
  via named devices from a platform descriptor (`--platform`), subtype
  trap-21 on hosted.
- Voice calibration: chapter 1 (post-review) is the reference.

## The non-rot contract

This book has the same discipline as the toolchain:

- **Green labs** compile, run, exit 0, and emit the `S\n` completion marker.
  The transcript in the text is real.
- **Red labs** must be **rejected at compile time** with the exact error code
  quoted in the text.
- **Engine labs** compile cleanly and trap at run time with the exact trap
  code quoted in the text (never emit the marker).
- Every lab section **embeds the full `.mod` source inline** in the chapter;
  the identical file lives under `labs/`. Labs are self-contained on the
  page.

Every lab lives as a real file under `labs/`, and CI (to be wired: a
`guards.sh`-style check) fails any chapter with no green or no red lab, any
green lab that stops compiling, and any red lab that starts compiling. The
book cannot silently drift from the compiler.

## Layout

```
devdocs/book_v3/
  README.md                       ← this file: conventions + TOC
  ch01-the-rigor-is-the-engine.md ← chapters, one file each
  labs/chNN/…                     ← lab fixtures, one file per lab
      green-NN-<name>.mod         ← must compile & run clean (marker, exit 0)
      red-NN-<name>.mod           ← must be rejected (code cited in chapter)
      engine-<name>.mod           ← must compile, then trap (code cited)
```

## Verified command forms (hosted target)

From the repository root (see Appendix C for a standalone-project layout):

```console
# build + direct run (shows the program's own output)
tyu build devdocs/book_v3/labs/ch01/green-01-hello.mod \
    --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
    --out-dir=build/green-01
./build/green-01/image.elf

# harness run (consumes output, checks the marker, renders a verdict)
tyu run devdocs/book_v3/labs/ch01/green-01-hello.mod \
    --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
    --out-dir=build/green-01
```

## Author grammar (verified against the current compiler)

Facts every chapter draft must obey; the manual (`tyu-technical-manual.md`)
lags the parser in places, and the code wins:

- **Word-signature clauses (current syntax):**
  - `needs [ … ]` — precondition predicate (the old `requires [ … ]` is a
    migration-hint parse error, E2195).
  - `ensures [ … ]` — postcondition predicate (unchanged).
  - `requires { … }` — capability set (brace form).
  - `performs { … }` — effect set (brace form; any clause order accepted).
- **Contract predicate shape (E3310 if violated):** the `needs`/`ensures`
  quotation must end with **exactly one bool on top** (net +1 relative to the
  word's declared ins/outs); inputs that survive must keep their declared
  types. Idiom for `( i64 -- )`: `needs [ dup 0 >= ]`. Two-sided range
  *contracts* in one clause need rot/over-style words the current builtins
  don't ship — range limits belong in **subtypes** (ch. 3), which the checker
  range-traps at word boundaries automatically.
- **Hosted entry ABI (E1018 if violated):** `main` must return exactly one
  value — the process exit code: `: main ( -- i64 ) … 0 ;`.
- **Completion marker:** clean programs emit `"S\n"` (via the platform's log
  word) before pushing the exit code. `tyu run` verdicts observed:
  - clean → silent (exit 0);
  - trap → `tyu: NO_COMPLETION — exited with code <trap> but no `S\n` marker`;
  - wrong native exit code with marker → `tyu: EXIT_MISMATCH — native exit code N != 0`.
- **Trap codes** (hosted exit status = trap code): 10 `STACK_OVERFLOW`,
  20 `CONTRACT_FAIL`, 21 `SUBTYPE_FAIL`, 22 `ASSERT_FAIL`, 23 `UNREACHABLE`.
- **Hosted I/O:** `platform.io.log ( str -- )` from
  `sysroot/x86_64-unknown-linux-gnu/platform/linux.*` (imports:
  `import platform/linux { platform.io.log };`). Channels (`platform.channel`)
  exist hosted-only; bare-metal `testio` words do not exist on hosted.
- **Builtins available in ch. 1–2:** `dup drop swap` `== != >= <=` `and or not`.
  Arithmetic (`+ - *`) comes with module imports/Core per fixture usage.

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

- **Hosted dynamic-mode entry is missing**: `tyu build --mode=dynamic
  --target=x86_64-unknown-linux-gnu` fails with "dynamic runtime entry
  unit .../dynamic_entry.asm is missing" — though the pack half works and
  produces a valid `.lmod`. The field-update labs therefore run on the
  QEMU track (and its cargo suites).

- **ISR budget error masked in the production path**: a handler whose peak
  exceeds its binding-site budget reports `E5030` via `langc --emit=ir`,
  but through `tyu build` the same program surfaces as `E7001` (missing
  word: main) — the typecheck abort masks the real verdict. Red lab 11.5
  uses the inspection path; the masking is worth a toolchain fix.
- **Hosted images with `@interrupt` bindings are not runnable** (they
  build, then crash at startup) — bindings are metal; green lab 11.1 is
  build-only by design.
- **ARM ISR runtime fixture is FIXME/opt-in**: `isr_lock_atomicity.mod`
  (lm3s6965evb SysTick) links and runs but its interrupt "doesn't fire in
  time" per the tree's own comment — chapter 11's runtime half is the
  documented open edge.
- **Hosted `tyu run --platform=linux-x86_64-hosted` fails**: the pack's
  metal startup is `linux-x86_64-hosted.asm` but the builder assembles a
  fixed `runtime` unit (`runtime/./runtime.asm` not found). Hosted MMIO
  over the descriptor's simulated devices is therefore not currently
  runnable; chapter 8 uses the QEMU track.
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
