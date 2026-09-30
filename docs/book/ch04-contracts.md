# Chapter 4 — Contracts: Promises the Machine Reads

> A type says what a value *is*.
> A contract says what must be *true*.
> Only one of them fits in a comment.
>
> — workshop wall

## 4.1 The Practice

> **The Practice.** *A contract is a checked program about a claim: the
> checker verifies the claim's shape at compile time, the runtime verifies
> the claim's truth at the boundary, and both verifications apply to every
> caller, present and future.*

Chapter 1 showed a `needs` clause working; chapter 3 showed ranges living in
types. This chapter finishes the idea — both halves of the promise, the
shape rules that make promises checkable, and the craft of writing promises
worth checking.

## 4.2 What contracts add to types

A type bounds what a value *can be*: a `Percent` is an integer in 0..100, a
`State` is one of its variants. But most important claims are not about one
value's range — they are about **relationships**: *the result is never
negative*, *this buffer is at least as long as that count*, *the motor is
stopped whenever the lid is open*. No type expresses "the result is never
negative," because negativity is not a kind of value — it is a *fact that
must hold at a moment in time*.

Tyu's words for those facts are the two halves of a contract, and they
divide a word's life exactly in two:

- **`needs [ … ]`** — the **precondition**. A claim about the inputs,
  checked *after the call, before the body runs*. The check lives in the
  callee, so every caller that ever exists is held to it.
- **`ensures [ … ]`** — the **postcondition**. A claim about the results,
  checked *after the body runs, before the word returns*. The check lives in
  the word itself, so a word cannot break its own promise quietly.

The division of labor is the classic one, and it is worth memorizing as an
obligation pair: **the caller must satisfy `needs`; the word must satisfy
`ensures`.** When a call misbehaves, the trap names which side broke —
the check that failed names its clause by position: before the body, a
precondition died; after it, a postcondition did.

Both clauses are written in the same style as everything else — a
quotation, pushed programs, stack words. There is no separate specification
language to learn, because the contract *is* a small program. That choice
is the chapter: since a contract is a program, it can be checked like one.

## 4.3 The shape rules: the checker checks the checks

What does the checker verify about a contract? Not its truth — truth belongs
to run time. It verifies the contract's **shape**, three rules that make a
predicate a well-formed claim:

1. **Exactly one boolean left on top.** The predicate runs on the declared
   inputs and must end with one `bool` — the verdict — and nothing else.
   Net effect: +1, top of stack.
2. **The inputs survive.** Below that boolean, the declared inputs must
   still be present with their declared types. A predicate that eats or
   replaces its subject is not a claim about it.
3. **No effects.** A predicate is a question, and questions do not log, send,
   allocate, or trap their way to an answer.

Break a rule and the rejection is immediate and coded:

| Code  | Broken rule                          |
|-------|--------------------------------------|
| E3310 | verdict count wrong (not one bool on top) |
| E3311 | the final value is not a boolean      |
| E3312 | the inputs were modified or lost      |

Read the three rules once more and notice what they buy: a contract
*cannot be vague*. It must compute one boolean from the values it claims
about, without side roads. Vagueness is the natural habitat of the
un-enforced promise; the shape rules evict it. (One boundary, stated in the
workbench note: the checker enforces the *shape*, never the *usefulness* —
that part is craft, and §4.6 is where it lives.)

## 4.4 The range idiom

Chapter 1 needed a range check — "zero or more" — and one comparison
sufficed. Two comparisons stacked, and the shape rules start to bite. The
naive reading of a two-sided check *looks* right and is flat wrong:

```tyu
needs [ dup 0 >= swap 100 <= and ]   # reads perfectly, checks nothing
```

Trace it as a pencil simulation: `dup 0 >=` consumes the input and leaves
one boolean; the input is *gone* before `swap 100 <=` runs, and the final
`and` eats even the booleans. Net effect: zero — no verdict, no inputs. The
checker rejects it with E3310 (Lab 4.4), and the rejection is the lesson: a
contract is stack code, and stack code that reads well is not the same as
stack code that checks well.

The working idiom uses `if` — chapter 2's decision word — as a
*combinator*: ask the first question; if it holds, ask the second; if not,
the answer is already `false`. One line, and every clause of it earns its
place:

```tyu
: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ]
  as i64 ;
```

Read it against the three shape rules and watch them all hold. From the
input: `dup 0 >=` copies the input, asks "zero or more?", and leaves the
input below one boolean. `if` consumes that boolean and runs one plan —
both plans start from the surviving input and end with that input plus one
boolean: the true branch asks "100 or less?"; the false branch, having
already failed, answers `0 0 ==` — false — without touching anything. One
verdict on top, input intact below, no effects. And the *meaning* is the
range: pass `0`, `50`, or `100` and the word runs; pass anything outside —
Lab 4.1's companion — and the runtime stops the program with
`NO_COMPLETION — exited with code 20`.

Why `0 0 ==` for the false branch? Because the answer to "is it in range?"
is already settled — no — and the branch must still produce a boolean
without disturbing the input. It is the stack idiom for *"already lost"*.
The style reads slightly foreign once and then forever after reads as
exactly what it is.

## 4.5 ensures: the promise about results

A postcondition is the mirror of a precondition: same quotation, same shape
rules, different moment. The predicate is compiled *after the body*, runs
against the results, and must leave one boolean on top:

```tyu
: bump ( i64 -- i64 )
  ensures [ dup 0 >= ]
  1 + ;
```

`bump` claims: *whatever you hand me, my result is zero or more.* Read the
clause as stack code against the result: copy it, ask "zero or more?" — one
boolean, input preserved, all three rules kept.

Now call `bump` with `-5`. The body runs — that is what bodies do — and
produces `-4`. The postcondition asks its question about `-4`, receives
`false`, and the runtime stops the program: `CONTRACT_FAIL`, exit 20, no
marker. The word broke its own promise, and the machinery named the moment.
Lab 4.3 is that trap, verbatim; Lab 4.2 is the same word kept.

The obligation pair is what makes this more than two assertion slots. A
caller of `bump` may *rely* on the result being non-negative — the check is
not advice, it is a guarantee the machine holds the word to. In exchange,
`bump` may rely on nothing about its input at all. Every contract moves a
fact from "someone must remember" to "the machine will say" — and each move
shrinks the space where the 3 a.m. bugs live.

## 4.6 The craft of checkable promises

The machinery checks shape. Craft supplies content. Three habits, in the
order they matter.

**Ask a real question.** `needs [ true ]` has perfect shape — one boolean,
no effects, nothing touched — and proves nothing. It compiles (verified),
runs, and guards like a smoke detector painted on the wall. The same
theater wears costumes: a comparison repeated from the type system, a
clause that restates the body instead of constraining it. After each
clause, ask the only question that matters: *what call does this reject?*
A contract with no rejecting call is a comment in brackets.

**Keep the dialect arithmetic.** Comparisons, logic, addition — the closer
a predicate stays to grade-school arithmetic over the values it is about,
the more readers trust it and the more tools can reason about it. This is
the provability ladder from chapter 1, now with a foothold: a future
milestone of the project is *static discharge* — proving preconditions
before the program runs — and the discharge tooling will be built for
exactly the dialect these chapters teach. Linear arithmetic, no effects,
values in view: write in that dialect now, and tomorrow's prover inherits
the specifications as-is.

**Bind at the boundary.** The check lives in the callee — chapter 1's
design, worth re-choosing here. A check written by the caller protects one
caller; a `needs` clause protects the word, in every context it will ever
be called from, including the one written by next year's teammate under
deadline. Contracts are interface, not hygiene.

## 4.7 The knobs

Contract insertion is a build decision, not a language keyword — the same
profile philosophy as everything else. The compiler flag is
`--checks=off|contracts|all` (default `all`). The claims survive the flag:
turning checks *off* does not delete the promises, only their enforcement —
visible in the IR, where the emitted check is one instruction:

```console
$ langc --emit=ir labs/ch04/engine-ensures-violated.mod \
        --target=x86_64-unknown-linux-gnu --sysroot=sysroot | grep -A1 cmp_ge
    cmp_ge
    trap_if_false CONTRACT_FAIL
$ langc --emit=ir --checks=off labs/ch04/engine-ensures-violated.mod \
        --target=x86_64-unknown-linux-gnu --sysroot=sysroot | grep -c contract
0
```

The production rule of thumb for embedded work: develop under `all`, ship
under the profile the platform's budget allows, and never let a check's
absence quiet a claim the design still makes. (`tyu` does not expose the
knob today — it is a `langc` flag; a profile hook is roadmap, not present.)

---

## Labs — Chapter 4

### Lab 4.1 — The two-sided range *(green)*

```tyu
# labs/ch04/green-01-two-sided-range.mod — Lab 4.1 (green)
# The two-sided range contract in one needs clause: the first comparison
# guards, the if combines, the input survives, one bool remains.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Range;
import platform/linux { platform.io.log };

: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ]
  as i64 ;

: main ( -- i64 )
  0 set-range drop
  50 set-range drop
  100 set-range drop
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Three calls, both edges included, silence from the harness. Then change
`50` to `150` and rerun: the same word that accepted the boundaries traps
with `CONTRACT_FAIL` (exit 20). One clause, two comparisons, and the
*whole* range is now the word's property.

### Lab 4.2 — The promise about results *(green)*

```tyu
# labs/ch04/green-02-ensures.mod — Lab 4.2 (green)
# An ensures clause is checked after the body, against the outputs:
# bump promises its RESULT is zero or more.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Bump;
import platform/linux { platform.io.log };

: bump ( i64 -- i64 )
  ensures [ dup 0 >= ]
  1 + ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  5 bump 6 == check
  0 bump 1 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean. Then answer before experimenting: which *side* owns the
obligation here — the caller or `bump`? Change the body to `1 -` and
predict the smallest input that turns the program into a trap.

### Lab 4.3 — The broken postcondition *(engine)*

```tyu
# labs/ch04/engine-ensures-violated.mod — Lab 4.3 (engine)
# bump's postcondition is checked after the body: -5 becomes -4, the
# promise about the result breaks, CONTRACT_FAIL.
# Expected: compiles cleanly; traps at run time with CONTRACT_FAIL
# (trap code 20, hosted exit status 20); never emits the marker.
module EnsuresBroken;
import platform/linux { platform.io.log };

: bump ( i64 -- i64 )
  ensures [ dup 0 >= ]
  1 + ;

: main ( -- i64 )
  -5 bump drop
  "S\n" platform.io.log
  0 ;
export { main };
end;
```

```console
$ tyu run devdocs/book_v3/labs/ch04/engine-ensures-violated.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch4-engine
tyu: NO_COMPLETION — exited with code 20 but no `S\n` marker
```

Chapter 1's engine demo broke a *precondition* — the caller lied. This trap
is the other half: the word lied about its result, and the postcondition
caught it *after the body ran*. Same verdict, different culprit; the
obligation pair made visible.

### Lab 4.4 — Reads well, checks nothing *(red)*

```tyu
# labs/ch04/red-01-flat-sandwich.mod — Lab 4.4 (red)
# This predicate READS like a two-sided range check, but its stack math is
# flat: dup+1, 0+1, >=-1, swap, 100+1, <=-1, and-1 = net 0, no bool left.
# Expected: rejected at compile time — E3310 (predicate must end with
# exactly one bool on top).
module FlatSandwich;

: set-range ( i64 -- i64 )
  needs [ dup 0 >= swap 100 <= and ]
  as i64 ;

: main ( -- i64 ) 50 set-range drop 0 ;
export { main };
end;
```

**Why:** each word in that clause is doing exactly what the English reads —
and the *total* consumes its own subject and both verdicts. The shape rules
do not grade reading comprehension; they count what is left on the stack.
The working form is §4.4's `if` idiom.

### Lab 4.5 — The verdict that is not a verdict *(red)*

```tyu
# labs/ch04/red-02-predicate-not-bool.mod — Lab 4.5 (red)
# The predicate ends one value on top — but it is an i64, not a bool.
# Expected: rejected at compile time — E3311 (contract not bool).
module NotBool;

: f ( i64 -- )
  needs [ dup ]
  drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** one value on top, net +1, input preserved — two of the three shape
rules pass, and it is still not a claim, because the final value is an
`i64`. A predicate ends in a question's answer, and `bool` is the only type
answers come in.

### Lab 4.6 — Postconditions obey too *(red)*

```tyu
# labs/ch04/red-03-ensures-shape.mod — Lab 4.6 (red)
# Postconditions follow the same shape rules: this one leaves nothing
# (net 0). Expected: rejected at compile time — E3310.
module EnsuresShape;

: f ( i64 -- i64 )
  ensures [ 0 >= ]
  1 + ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `0 >=` consumes the result to ask about it — the mirror of Lab
4.4's crime, on the other side of the body. The fix is the copy-first idiom:
`ensures [ dup 0 >= ]`, which Lab 4.2 already runs.

### Post-mortem — contract theater *(optional — for readers with C or embedded scars)*

Every long-lived C codebase grows a collection of assertions that cannot
fail: `assert(ptr != NULL)` on a pointer dereferenced three lines earlier;
checks compiled out by `NDEBUG` in the release build, so the shipped
binary runs on faith; "validation" functions that return `true` from every
path because returning `false` once crashed a customer demo and nobody had
time to trace it. The theater is never intentional — it accretes, one
comforting no-op at a time, until the codebase believes it is checked.

The shape rules are Tyu's structural answer: a contract must compute one
boolean from live inputs, so *some* claim is being made — the machinery
will not accept a shrug. What the machinery cannot supply is the second
half, and this chapter's craft section says so directly: a contract that
rejects no call is decoration. The review habit to take from both halves —
*machinery checks shape; humans check content* — is exactly the division
the obligation pair already teaches.

### From the workbench

Why are contracts ordinary programs instead of a specification language?
One mechanism, three payoffs. There is nothing new to learn — the predicate
uses the words chapter 2 taught. There is nothing new to trust — the
checker simulates a predicate exactly as it simulates any word, rules and
error codes and all. And there is nothing to translate — when the static
discharge tooling arrives, the prover will read the same program the
runtime runs today, because they are the same program. A separate spec
language would have to be parsed, checked, and *proven equivalent to the
code it describes*; Tyu skips the equivalence proof by refusing to have two
things.

### Exercises

1. **Read the shape.** For each predicate, decide *before compiling* which
   rule fails and with which code: `[ dup dup 0 >= and ]`,
   `[ 0 drop ]`, `[ dup 0 >= dup ]`. Confirm with `langc`, then write one
   sentence per predicate naming what it was trying to claim.
2. **A two-sided contract of your own.** Declare `subtype Millivolts = i64
   range 0 .. 5000` (chapter 3) and write `: set-v ( i64 -- i64 )` with a
   two-sided `needs` clause guarding that range, using the `if` idiom.
   Verify both edges pass (`0`, `5000`) and one value outside traps.
3. **The obligation pair, in prose.** A teammate's code calls `bump` with
   an arbitrary integer and then stores the result "knowing" it is
   non-negative. Using the words *precondition*, *postcondition*, *caller*,
   and *callee*, write the two-sentence contract explanation: what each
   side owes, and which trap fires when each side breaks its word.

## 4.8 Contract obligations and elision

A contract is also a *proof obligation* (chapter 3 introduced the range
half of this story). Every `needs`/`ensures` site is recorded by the
compiler — the callee's own `needs` (`contract-pre` in the callee's
artifact), every call site to a contracted word (`contract-pre` in the
caller's artifact), and every `ensures` epilogue (`contract-post`) — and
each record can be *discharged*:

- a **constant** verdict discharges: a word whose postcondition is proven
  on its results elides its runtime trap under `--checks=undischarged`
  (`tyu build`, the default);
- a **named predicate** travels across modules: `needs [ pct-in-range ]`
  in a `.def` names the callee-side predicate, and its compiler-computed IR
  (`<Module>.obl.json` facts) is what a caller's discharge is checked
  against — a `.def` naming a predicate the callee's artifact no longer
  documents is a hard `E6413` stale-interface error, never a silent open;
- a **policy** gate decides: `--verify-policy=no-open` fails the build
  (`E6410`) listing every open contract obligation — the repo's own corpus
  adopts it per-module with a reviewed allowlist.

One elision boundary is a policy, not a proof: under `module-loading`, a
dynamically-exported contracted word is a *runtime surface* no build-time
discharge may remove, so its contract checks are retained and the report
lists the site under `retained` with the reason.

---

*Next: chapter 5 — Effects and Capabilities. The two-sided law of what
programs *do*: `performs` and `needs {…}`, the contexts that grant and
forbid, and why a suspension is something a word must declare and a handler
must discharge.*
