# Chapter 15 — The Machine as Witness: Emulation in Verification

> A test that cannot disagree with you is not a witness.
> The corpus is the cross-examination.
>
> — workshop wall

## 15.1 The Practice

> **The Practice.** *Every semantic claim the toolchain makes is replayed
> by at least two independent machines, and the machines are not allowed to
> agree by construction: one proves, one executes, and a shared corpus of
> observations is the only thing either is allowed to say. When they
> disagree, the corpus wins and the code is wrong — and this has already
> happened, catching a real bug.*

Chapter 14 put proofs in the developer's hands. This chapter is the other
half of the trust story, and it is not about proofs at all: it is about
**emulated execution as a witness**. The claim under examination is always
the same — *"this operational semantics says what the language does"* —
and the examination is always the same shape: run the claim through an
independent implementation against a shared corpus of observations, and
let the corpus refuse whichever side has drifted.

The chapter's rule, which the whole toolchain observes:

> **Zero divergence is the only passing grade.** A witness that is allowed
> occasional disagreement is not a witness; it is a screensaver.

## 15.2 The conformance corpus: `tyu.vec/1`

The semantics table of the language — every operation's transfer, over
every recognized target — is rendered once by the toolchain and consumed
twice. The Rust verifier executes it one way; the Lean port's abstract
transfer engine (chapter 14's other machine) executes it another. Between
them sits the **conformance corpus**: `tyu.vec/1` documents, per target,
every observation a semantics row must reproduce — boundary classes get
extra vectors, and the corpus is generated, not hand-written, so it grows
with the language instead of rotting behind it.

The board's corpus is small enough to hold in one glance — here is one
full vector, the extreme of what a constant can be:

```json
{
 "id": "boundary/const-i64-max",
 "class": "boundary:i64-max",
 "row": "const_i64",
 "ops": "const_i64 9223372036854775807",
 "expect": { "head": "def-false", "top": "[9223372036854775807]" }
}
```

The conformance runner reads the corpus, replays every `ops` string
through its own transfer, and compares `head` and `top` **byte-exactly**.
Fifty-nine vectors on this target, hundreds across the four triples,
**zero divergence** is the gate. The gate runs in CI as `ci/port.sh` —
and its own output lists what a recognized port must survive:

```console
$ bash ci/port.sh lean
    port.sh: P14 re-derivation gate green (rederive selfcheck + corpus +
             automation-only proven)
    port.sh: PORT GATE GREEN (conformance + axiom audit + stackmeta +
             P9.3 fragment corpus + P5 gen drift + P6 pipeline + P7.1
             harvest + P9 source surface + P10 automation + P12
             bundles/bands + P13 refinement + P14 re-derivation)
```

Every name in that parenthesis is a cross-examination that passed:
statement-pipeline drift, harvest tampering, the source surface, bundle
modeling, re-derivation agreement. A port that fails any element stops
being *recognized* — and an unrecognized port's verdicts are demoted to
attestations, not proofs. Recognition is continuous, which is the point.

## 15.3 The witness that earned its keep

Why obeyance to a corpus is worth the machinery is not an argument — it is
a war story, and it happened during this pipeline's own construction. The
Lean port's fragment corpus (`tyu.fragvec/1` — whole small programs, not
single operations) refused to agree with the port's transfer of `swap`:
the port had implemented it as the identity. The corpus said: the
authoritative row is `( a b → b a )`, the Rust engine agrees with the row,
and your transfer does not. The port was fixed; the corpora now agree.

The deeper fact is *which side was wrong*. The corpus did not know or
care — it is a deposition, not a judge. Any machine that drifts from the
deposition loses, whether it is the hobby backend or the proof port. The
same discipline powers the **re-derivation differential**: the port
re-runs the toolchain's interval analysis on generated programs and must
agree per-obligation — including agreement on the *trust class* — across
every CI run. Two implementations, one corpus, zero divergence, forever.

Lab 15.2 lets the reader commit the crime personally: tamper one
expectation in the corpus, and watch the runner refuse the document and
name the divergence:

```console
$ conformance --corpus /tmp/tampered/x86_64-unknown-none
DIVERGE: x86_64-unknown-none [boundary/const-i64-max]:
  got head=def-false top=[9223372036854775807,…]; want head=def-false top=[999]
RESULT: vectors=59 mismatches=1     (exit 1)
$ conformance --corpus crates/verifier/test-vectors/x86_64-unknown-none
RESULT: vectors=59 mismatches=0     (exit 0)
```

The refusal names the vector, the observed transfer, and the tampered
expectation. This is what "zero divergence" means mechanically: not a
dashboard, a diff.

## 15.4 The board as witness

The third emulation is the one this book has used since chapter 6: the
board itself, emulated by QEMU, executing what nothing else can check.
The proof pipeline's own boundary document — the TCB list the certification
package ships — draws the line precisely, and the board's side of the
line is populated by labs this book has already run:

| Claim                                          | Carried by                     | Witnessed on the board |
|------------------------------------------------|--------------------------------|--------------------------|
| data-stack bounds (`high`)                     | the `(net, high)` proof (ch. 6) | the measured high-water marker, asserted against the declared bound |
| the machine-code lowering                      | **nothing formal — empirical** | the execution suites: arithmetic, MMIO round-trips, ISR binding, signed/encrypted loads |
| the loader's field-update gates                | the transactional loader       | the signed/negative/encrypted suites (ch. 13) |

The middle row is the true one, and it is the reason the board matters:
*the compiler's lowering is not proven*. The verified path from source to
machine code ends at conformance vectors and QEMU runs — the same
combination AWS's s2n-bignum uses for hand-tuned assembly, and the same
reason seL4's proof chain ends at a validated binary rather than an
assumption. Empirical is not a slur; it is a scope statement. The TCB
document ships in every certification package so that no auditor has to
guess which claims are theorems and which are test results.

## 15.5 Three witnesses, one deposition

Set the chapter's machines side by side and the design reads as one
deposition taken three ways:

- The **corpus** deposes what the semantics says (rows, boundaries,
  programs) — and both implementations must repeat it exactly.
- The **differential** deposes that two independent analyses reach the
  same verdicts, per obligation, at scale.
- The **board** deposes that the compiled artifact behaves on the
  hardware path, within the scope the TCB admits.

None of the three is trusted alone; each covers the others' blind spots.
The proof (chapter 14) covers everything the corpus can express; the
corpus covers the proof port's drift; the board covers the lowering no
proof reaches yet. This is the same structure the research literature
arrived at the hard way — verified kernels, independent checkers,
machine-code verification — arrived at here in miniature, with the same
load-bearing wall: a small deposition both machines must repeat.

---

## Labs — Chapter 15

### Lab 15.1 — The port gate *(green)*

```console
$ cd verification/ports/lean && lake build Tyu conformance
$ .lake/build/bin/conformance --corpus \
      ../../../crates/verifier/test-vectors/x86_64-unknown-none \
      ../../../crates/verifier/test-vectors/x86_64-unknown-linux-gnu \
      ../../../crates/verifier/test-vectors/armv7m-unknown-none \
      ../../../crates/verifier/test-vectors/riscv32-unknown-none
$ cd ../.. && bash ci/port.sh lean
    port.sh: PORT GATE GREEN (conformance + axiom audit + …)
```

The first command builds the port's library and its conformance
executable; the second replays every vector of every recognized target;
the third is the blocking gate. Zero divergence, or the port is not
recognized.

### Lab 15.2 — Tamper with the deposition *(red)*

Copy a target's corpus, alter one expectation value, and run the
conformance runner against the tampered copy:

```console
$ conformance --corpus /tmp/tampered/x86_64-unknown-none
DIVERGE: x86_64-unknown-none [boundary/const-i64-max]:
  got head=def-false top=[9223372036854775807,…];
  want head=def-false top=[999]
RESULT: vectors=59 mismatches=1     (exit 1)
```

**Why:** the runner does not average, does not tolerate, does not
"mostly agree." One mismatched vector among fifty-nine is exit 1 with the
divergence named — vector id, observed transfer, claimed expectation. The
reader has now personally committed the only crime a witness exists to
catch.

### Lab 15.3 — The differential, on demand *(green)*

```console
$ cargo test --release -p verifier --test rederive_differential
test result: ok. 2 passed; 0 failed
```

The port re-executes the toolchain's interval analysis and must agree
*per obligation, including the trust class* — a `checked` that the port
would call `open` is a divergence like any other. In CI this runs across
generated programs at scale; on a laptop, the suite is the same
assertion in miniature.

### Lab 15.4 — The board, re-examined *(green, QEMU — already met)*

Chapters 6, 11, and 13's board suites are this chapter's third witness
under other names: the high-water marker asserted against the declared
bound (ch. 6), the signed/negative/encrypted field-update suites
(ch. 13), and the interrupt-binding gates (ch. 11, whose end-to-end
runtime remains the documented open edge). Re-run any one of them and
note what it witnesses that no corpus can: the compiled artifact,
executing.

### Post-mortem — the witness that agreed with itself *(optional — for readers with C or embedded scars)*

The C testing failure this chapter structurally refuses: the test suite
written by importing the implementation's own header and calling its own
helpers — so the test asserts what the code does, and passes forever,
including when the code is wrong. The genre's deadliest form is the
reference implementation used as the oracle for itself: two copies of one
bug, agreeing enthusiastically.

The corpus is the antidote, and its power is *independence plus
deposition*: the observations are generated from the semantics rows, not
from either implementation; they are committed, hash-pinned, and
byte-compared; and the two machines examining them were built by
different hands in different languages. The `swap` bug of §15.3 is the
proof the discipline works — the deposition did not care which
implementation was older, larger, or written by the language's author.
It said what `swap` does; one machine disagreed; the machine was wrong.

### From the workbench

Why byte-exact comparison instead of "close enough"? Because semantics
differences start small. An off-by-one in a boundary vector, a
sign-extension rendered inconsistently, a widening that differs at
`i64::MAX` — each is invisible to a tolerance and fatal to a proof that
reasons over the same rows. The corpus's expectations are *data*, pinned
and reviewed like goldens, so a semantics change that perturbs behavior
is a reviewed diff, not a discovery. When the research literature says
"independent checkers" and "conformance vectors," this chapter is what
those words cost: generated corpora, byte-exact replay, and a gate that
treats one divergence as total.

### Exercises

1. **Read a deposition.** Pick any vector from
   `crates/verifier/test-vectors/x86_64-unknown-none/index.json` and
   write, in one sentence, the semantic claim it pins — then find the
   semantics row it names and confirm the claim is a faithful reading of
   the row.
2. **The boundary class.** Count the vectors whose `class` begins with
   `boundary:` and name the three boundaries they bracket. Why does the
   corpus over-sample boundaries relative to ordinary rows?
3. **Two witnesses, one lie.** Suppose the Lean port and the Rust engine
   agreed with each other but both drifted from the corpus. Which lab
   catches it, and why does the corpus's independence from both
   implementations matter for that answer?

---

*Next: chapter 16 — Tyu's Effect on Thinking. The epilogue.*
