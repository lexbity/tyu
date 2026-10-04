# Chapter 14 — Proving the Program: The Developer-Proof Pipeline

> The kernel does not know who wrote the proof.
> That is the whole security model, and the whole economy.
>
> — workshop wall

## 14.1 The Practice

> **The Practice.** *A contract you wrote is a claim. A proof makes it a
> theorem. The kernel — a small, independent checker — is the only judge;
> the toolchain's job is to generate the claims, render them faithfully,
> and never let an unproven claim be mistaken for a proven one.*

Chapter 1 made a promise with an expiration date: *"write contracts the
way the prover will want them — when the tooling lands, your chapter 4
habits will already be proofs in waiting."* The tooling has landed. This
chapter is the discharge of that promise, and it changes one word in the
book's oldest sentence: a contract is no longer *checked at run time* —
it is a **theorem**, kernel-verified, harvestable into the build, and
loadable as part of a module's evidence.

## 14.2 The pipeline, end to end

The flow for a module with contracts is three commands:

```console
$ tyu proof init --dir=. P2.mod          # scaffold proofs/ once
tyu: scaffolding proofs/ in '…/proofs'
$ tyu build P2.mod --verify-tool=lean …  # generate, elaborate, harvest
tyu: proof statements P2: 1 rendered, 1 omitted, 1 unproven
verify: 2 obligations — 1 discharged, 0 assumed, 1 open
$ tyu proof fill --dir=. P2.mod          # automation proposes candidates
tyu: proof fill: 1 candidate file(s) written to '…/proofs/candidates'
    (unreviewed; review by removing the `-- tyu:candidate obligation=`
    marker line)
```

What happened, in order. The compiler extracted **obligations** — machine-
readable claims about the module (`tyu.obl/v2`): subtype range promises,
contract pre/post, each with a span, an *intent string*, and the word's
IR. Each obligation was canonically encoded and **hashed** — the
`statement_hash` that makes the claim un-editable in passing. The
renderer then wrote the claims as Lean `Prop` definitions into the
generated package (`.tyu-verify/lean/Gen/P2.lean` — regenerated every
build, digest-checked at harvest, E6418 if anyone edits it):

```lean
def stmt_P2_take_subtype_range_0 : Prop :=
  Tyu.Gen.Stmt.inInputRange 0 (0) (100)
```

The developer's job is to write, in `proofs/P2.lean` (a VCS-tracked,
developer-owned file the toolchain never edits), a theorem *of exactly
that type*:

```lean
theorem obl_P2_take_subtype_range_0 :
    (Tyu.Gen.Corpus.P2.stmt_P2_take_subtype_range_0) := by …
```

Weakening the claim would require editing the generated definition — which
the next build regenerates and the digest catches. **The type system is
the first binding; the digest is the second; neither negotiates.**

`lake build` elaborates everything; the **harvest** step then binds each
theorem to its statement in the kernel environment, runs the **axiom
audit** (the only permitted axioms are the benign logical set — `sorry`
and friends are fail-closed errors), checks **assumption closure** (no
proof may rest on an obligation nobody discharged), and emits
`tyu.verdicts/v2`: per-obligation status, **trust class** (`proof`,
`checked`, `assumed`, `open`), **method** (`certificate` — a theorem;
`rederive` — the port re-running the toolchain's own analysis), certifier
identity, and surface provenance. The book's oldest rule — fail closed —
holds here too: a harvest that cannot verify its bindings emits E6416 and
*every obligation goes open*, checks retained.

## 14.3 The dialect is load-bearing

Chapter 4 asked for contracts in arithmetic; chapter 9 factored one into
a helper; and here the two habits collide with the pipeline —
instructively. Try to run the chapter 9 factored contract (its `needs`
clause calling `bounded-by`) through the pipeline's obligation pass:

```console
$ tyu build Range.mod --verify-tool=lean …
error[E3312]: typecheck error
tyu: build error: build: pass-1 (obligations) failed on 'Range.mod'
```

E3312 — the contract-modifies-its-inputs rule, now enforced with the
origin tracking the proof pipeline needs: a predicate that computes
through helper calls cannot have its inputs' provenance tracked, so the
extraction refuses it. The chapter 4 *inline* idiom passes the same pass.
The lesson has sharpened: **the arithmetic dialect is not a style
preference — it is the pipeline's input language.** A contract that
wants to be a theorem must be arithmetic over the values in view.
(Lab 14.4 runs the refusal.)

## 14.4 The automation, and the marker

One obligation class was already discharged in §14.2's transcript without
any developer touching a theorem — `1 discharged` — by the toolchain's
interval analysis, recorded as trust `checked`: automation's plain
grade. The port's re-derivation (chapter 15's differential) can upgrade
such discharges to `proof` without developer effort.

For everything else there is **`tyu proof fill`** — the agentic half of
the pipeline, and the reason this chapter exists in a book for students.
Fill invokes the port's automation library per unproven obligation and
writes *candidate proof scripts*:

```lean
-- tyu:candidate obligation=P2::take::subtype-range::0
-- candidate theorem (unreviewed, automation-generated; the kernel gates it).
-- fill-budget=10s; remove the marker to acknowledge review.

theorem obl_P2_take_subtype_range_0 :
    (Tyu.Gen.Corpus.P2.stmt_P2_take_subtype_range_0) := by
  tyu_auto
```

Read the marker line twice, because it carries the chapter's ethics. The
candidate is **unreviewed, automation-generated** — and the pipeline's
treatment of it is neither suspicion nor trust: the kernel checks it like
any theorem (an invalid candidate fails elaboration, full stop), and the
*attribution* records it as `authored: "candidate"` so that the human
signing the deploy knows what they are signing. Promotion to reviewed is
a developer edit — move the theorem into `proofs/P2.lean` (imports to the
top; Lean rejects mid-file imports, Lab 14.2's learned-the-hard-way
moment) and remove the marker. The verified e2e suite pins the whole
contract: a candidate verdict is `trust: proof`, `discharged`, and
`authored: candidate` — **the marker never licenses an unchecked label;
the kernel is the only gate.**

One environment note, stated as the book's contract requires: the
candidate flow above was verified through the port's own gate (where the
automation phase reports green, including *automation-only proven*
builds); on a bare system lean without the pinned toolchain, the
`tyu_auto` tactic's simplifier set can fail to close a goal that the
pinned environment closes. Use the pin (Appendix C), as the port gate
does.

## 14.5 What the pipeline refuses

Two rejections teach the system's truth-telling better than its successes.

**The anonymous promise.** A `needs [ … ]` clause written inline (no
named predicate) extracts as an obligation whose provenance is
*opaque* — and the renderer **declines to fabricate a statement** for it:

```console
tyu: proof statements B: 0 rendered, 1 omitted, 0 unproven
verify: 1 obligations — 0 discharged, 0 assumed, 1 open
```

The obligation stays open, the runtime check stays in the code, and the
report says *omitted* — the machine refusing to manufacture a proof
shaped object for a claim it cannot state (Lab 14.5).

**The tampered generation.** Harvest binds theorems to the *generated*
statements by digest; a hand-edit to the generated file — a weakened
claim, a renamed statement — is fail-closed:

```console
$ bash tamper/run-mutate-gen.sh "$PWD/harvest-fixture"
  tamper mutate-gen: harvest failed closed with an E6420 error document
```

The port's tamper suite tries the crimes so the reader does not have to:
mutate the Gen metadata (E6420), delete the theorem — every one fails
closed, never "best effort" (Lab 14.6).

## 14.6 The package, briefly

A verified build's evidence can be carried with the module: `tyu deploy`
under the `proven` policy refuses any image whose modules carry open
obligations or unmodeled platforms (E6510), and the certification package
(`.tyucert/`) ships the obligations, the verdicts, the canonical
statements, *the proofs themselves*, the axiom audit, and a TCB document
listing — by name — every trust boundary that remains structural rather
than theorem. Chapter 13 signed the container; this chapter signed the
claims. The auditor's job is to read; the machine's job was to refuse.

---

## Labs — Chapter 14

### Lab 14.1 — Claims, generated *(green)*

Module `labs/ch14/p2/P2.mod`:

```tyu
# labs/ch14/p2/P2.mod — Lab 14.1 (green)
# One subtype in parameter position: the pipeline renders its range
# promise as a statement for the developer to prove.
# Expected: builds through the Lean pipeline; one statement rendered.
module P2;
subtype Percent = i64 range 0 .. 100;
: take ( Percent -- ) drop ;
: main ( -- i64 ) 50 as Percent take 0 ;
export { main };
end;
```

```console
$ tyu proof init --dir=. P2.mod
tyu: scaffolding proofs/ in '…/proofs'
$ tyu build P2.mod --verify-tool=lean --target=x86_64-unknown-none \
      --platform=x86_64-unknown-none --sysroot=sysroot \
      --out-dir=build/ch14
tyu: proof statements P2: 1 rendered, 1 omitted, 1 unproven
verify: 2 obligations — 1 discharged, 0 assumed, 1 open
$ cat .tyu-verify/lean/Gen/P2.lean
def stmt_P2_take_subtype_range_0 : Prop :=
  Tyu.Gen.Stmt.inInputRange 0 (0) (100)
```

**Notice** the candor in the counts: one statement *rendered* (the
subtype's range, in parameter position), one *omitted* (§14.5), one
*unproven* (nobody has proved the rendered one yet) — and one obligation
already discharged by the automation, without a theorem, at trust
`checked`.

### Lab 14.2 — The candidate *(green, the agentic lab)*

```console
$ tyu proof fill --dir=. P2.mod
tyu: proof fill: 1 candidate file(s) written to '…/proofs/candidates'
    (unreviewed; review by removing the `-- tyu:candidate obligation=`
    marker line)
$ cat proofs/candidates/P2__take__subtype_range__0.lean
-- tyu:candidate obligation=P2::take::subtype-range::0
-- candidate theorem (unreviewed, automation-generated; the kernel gates it).
…
theorem obl_P2_take_subtype_range_0 :
    (Tyu.Gen.Corpus.P2.stmt_P2_take_subtype_range_0) := by
  tyu_auto
```

Promote it — theorem into `proofs/P2.lean`, imports at the top, marker
kept until reviewed — and rebuild. The verified e2e suite pins the
contract the promotion exercises:

```console
$ cargo test --release -p tooling-tests --test candidate_e2e
test candidate_verdict_is_kernel_checked_and_attributed ... ok
```

Its assertions are the lab's answers: the candidate's verdict carries
`"trust":"proof"` (the marker never licenses an unchecked label),
`"status":"discharged"`, and `"authored":"candidate"` — attributed for
the signing human, gated by the kernel for everyone else.

### Lab 14.3 — The source surface *(green)*

```console
$ cargo test --release -p tooling-tests --test source_surface_e2e
test source_surface_worked_example_harvests_with_source_provenance ... ok
```

Obligations can be proven on either of two semantics — the IR, or the
pure source fragment — related by a machine-checked transcription
theorem. This suite proves a worked example **on the source surface**
and harvests it with `surface: "source"` provenance; the verdict records
which surface the certificate rests on, and the package names the
transcription theorem in its trust boundary.

### Lab 14.4 — The pipeline refuses the helper dialect *(red)*

```tyu
# labs/ch14/range/Range.mod — Lab 14.4 (red)
# The chapter 9 factored contract: a needs clause that computes through a
# helper word. The obligation pass refuses it — predicates must be
# arithmetic over values in view. Expected: E3312 at pass-1.
module Range;
: bounded-by ( i64 i64 -- i64 bool )
  => hi dup hi <= ;
: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ 100 bounded-by ] [ false ] if ]
  as i64 ;
: main ( -- i64 ) 50 set-range drop 0 ;
export { main };
end;
```

```console
$ tyu build labs/ch14/range/Range.mod --verify-tool=lean …
error[E3312]: typecheck error
tyu: build error: build: pass-1 (obligations) failed on 'Range.mod'
```

**Why:** the same word built *without* the pipeline compiles — the
obligation pass is stricter because extraction must track the inputs'
provenance through the predicate, and a helper call breaks the trail.
The chapter 4 inline idiom passes. The prover's dialect is not
negotiable; this is what "write contracts the prover will want" means
mechanically.

### Lab 14.5 — The machine declines to imagine a claim *(red)*

```tyu
# labs/ch14/anon/Anon.mod — Lab 14.5 (red)
# An anonymous inline needs predicate: the obligation exists, but its
# provenance is opaque, and the renderer declines to fabricate a
# statement. Expected: 0 rendered, 1 omitted; the obligation open with
# its runtime check retained.
module Anon;
: f ( i64 -- ) needs [ dup 0 >= ] drop ;
: main ( -- i64 ) 5 f 0 ;
export { main };
end;
```

```console
$ tyu build labs/ch14/anon/Anon.mod --verify-tool=lean …
tyu: proof statements Anon: 0 rendered, 1 omitted, 0 unproven
verify: 1 obligations — 0 discharged, 0 assumed, 1 open
```

**Why:** the pipeline could have rendered *something* provable — a
tautology shaped like the claim. Instead: omitted, open, and the runtime
check still in the binary. Compare with chapter 4's tautology
post-mortem: this is that refusal, institutionalized.

### Lab 14.6 — Tamper with the generation *(red)*

```console
$ cd verification/ports/lean/tests
$ bash tamper/run-mutate-gen.sh "$PWD/harvest-fixture"
  tamper mutate-gen: harvest failed closed with an E6420 error document
$ bash tamper/run-delete-theorem.sh "$PWD/harvest-fixture"
```

**Why:** the harvest binds each theorem to the *generated* statement it
was given — rename the statement in the generated metadata (the first
script) or delete the developer's theorem (the second), and the harvest
fails closed with a coded error document rather than harvesting a best
guess. The weakened-claim insider, the broken-pipeline optimist, and the
deleted-proof pessimist all meet the same wall.

### Post-mortem — proving the wrong thing *(optional — for readers with C or embedded scars)*

The research literature calls it specification drift: a mathematically
valid theorem about the wrong formal definition — the proof is airtight
and irrelevant. In industrial proof deployments this is *the* failure
mode, because the expensive part (the proving) works, and the cheap part
(keeping the statement true) is nobody's job.

This pipeline makes it the machine's job, three times over. The
statement is a generated definition, so the developer proves a *type*,
not a goal — the type system is the first binding. The generation is
digest-checked at harvest, so an edited claim is a build break
(E6418/E6420) — the second binding. And the statement's canonical hash
rides into the verdicts and the certification package, so a verdict from
an older claim is *stale*, not stale-ish (E6421 at consumption) — the
third binding. Three mechanisms, one refusal: the proof is about what
the compiler said, or it is nothing.

### From the workbench

Why is the kernel the only gate — why does the pipeline not also vet
proofs "for quality"? Because any additional gate is additional trusted
code, and trusted code is the attack surface. The research lineage is
explicit: separate proof *search* (tactics, automation, language models —
all untrusted, all welcome) from proof *validation* (a small kernel —
the only thing that must be correct). The pipeline's automation writes
candidates; the kernel decides. The axiom audit keeps the logic clean;
the statement binding keeps the logic *about Tyu*; the closure check
keeps modular proofs from resting on air. Everything else — the intent
strings, the attribution markers, the candidate ratios in the deploy
report — is for the humans, who get exactly what the developer declared:
no more, and never less.

### Exercises

1. **Read the receipt.** Lab 14.1's report says `1 discharged` before any
   theorem exists. Using the trust table (§14.2), name the method and
   trust class of that discharge, and explain why it is not `proof`.
2. **The dialect audit.** Take chapter 4's two-sided range contract and
   chapter 9's `bounded-by` factoring. One passes the obligation pass;
   one is refused (Lab 14.4). Restate chapter 9's factoring grammar with
   this new rule included, and give the factored form that *would*
   pass — or prove to yourself that none exists yet, and what that
   costs.
3. **The marker's meaning.** A teammate deploys under `proven` with a
   report showing `authored: "candidate"` on twelve of forty verdicts.
   Using the words *kernel*, *attribution*, and *review*, write the
   three-sentence explanation of what is and is not being claimed — and
   which policy knob a fleet operator would set to refuse candidates
   outright.

---

*Next: chapter 15 — The Machine as Witness: Emulation in Verification.
The corpora, the differentials, and the board: how the toolchain
cross-examines its own semantics with machines that are not allowed to
agree by construction.*
