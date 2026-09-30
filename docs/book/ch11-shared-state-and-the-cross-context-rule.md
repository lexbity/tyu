# Chapter 11 — Shared State and the Cross-Context Rule: Interrupts

> The interrupt arrives in a microsecond.
> The race it creates ships for a year.
>
> — workshop wall

## 11.1 The Practice

> **The Practice.** *Shared hardware is proven, not prayed over: if an
> interrupt can reach a resource, every access everywhere is inside a lock,
> the lock lowers to whatever the platform needs, and the handler's own
> stack is budgeted at its binding. The race that ships for a year becomes
> a compile error that ships never.*

This is the first of the daring parts — the chapter where the disciplines
of ten chapters combine into the guarantee that justifies them. An
interrupt handler and mainline code share a device. In C this is the most
ordinary of dangers; in Tyu it is a proof obligation, discharged word by
word.

## 11.2 The race, and the shape that prevents it

Set the stage with chapter 5's pieces. A `resource` — storage that is
writable only inside its lock — and a handler bound to a timer vector:

```tyu
resource Counter : i64;

@interrupt(TIMER0) : isr ( -- )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
;
```

Two facts about that handler, both checked before the program exists.
First, its access to `Counter` sits inside the lock — the grant is
lexical, exactly as chapter 5 taught, and the ISR context is no exception.
Second, the *binding* — `@interrupt(TIMER0)` — is where the handler meets
its vector, and chapter 6's rule applies there: the handler's stack shape
is checked against a budget, so a recursive or unbounded handler is
rejected at the binding (E5040 was chapter 6's red lab; the budget
refusal is this chapter's Lab 11.5).

Mainline code touches the same resource the same way:

```tyu
: main ( -- i64 )
  Counter lock [ &!Counter @i64 drop ]
  …
```

That pair — handler and mainline, each inside its own lock — is the
*compliant shape*, and Lab 11.1 builds it clean. The lock does double
duty: it grants the write capability (chapter 5's two-sided law) and it
*lowers* to the mutual-exclusion mechanism the sharing actually requires.
That lowering is not one-size-fits-all; it is chosen per resource by the
cross-context analysis:

| Who reaches the resource            | The lock lowers to                    |
|--------------------------------------|----------------------------------------|
| main only                            | a compiler fence                       |
| main + ISR, single core              | interrupt mask / unmask                |
| main + ISR, multiple cores           | mask + hardware spinlock (`core(N)`)   |

The `core(N)` capability from chapter 5's capability table is the
multi-core escalation — a capability granted by the platform, declared in
the descriptor of the sharing.

## 11.3 The cross-context rule

Now the rule that gives the chapter its name, and the star red lab:

> **If a resource is reachable from any interrupt context, then every
> access to it — in every context, everywhere in the program — must be
> inside its lock, and the lock must be interrupt-aware.**

The reasoning is chapter 5's forbids-accumulate principle, run across
contexts instead of within one. `Counter` is reachable from the ISR; that
fact *poisons every unlocked access in the program*, because any of them
can be interrupted mid-update by code that mutates the same storage. The
check is whole-program — which is what makes it valuable — and Lab 11.4
shows its verdict. The compliant program of §11.2 with one lock deleted
from `main`:

```console
$ tyu build labs/ch11/red-03-shared-unlocked.mod …
error[E5031]: typecheck error
```

E5031 — `RESOURCE_SHARED_UNLOCKED`. The unlocked borrow in `main` is
one `@i64` long; the race it enables is the classic lost update: mainline
reads the count, the interrupt fires, the handler increments, mainline
writes its stale value — and the handler's increment never happened.
Desktop testing rarely catches it; the field always does. Here it is a
compile error whose citation is a rule about reachability.

Two quiet consequences of the rule, both design-shaping:

- **The lock's cost is visible.** A resource only `main` touches lowers to
  a fence — nearly free. A shared resource pays for real mutual
  exclusion. The type-and-effect system makes the cheap case *provably*
  cheap, so the expensive mechanism is only bought where the proof demands
  it.
- **Removing the ISR's access removes the rule.** If no interrupt can
  reach `Counter`, the resource is not shared, and main's unlocked access
  compiles. The rule tracks reachability, not ritual — exercise 1 asks
  the reader to walk both directions.

## 11.4 The handler's own discipline

The ISR context is a row in chapter 5's matrix, and its forbids are the
sharpest in the language. Lab 11.3: `platform.task.yield` inside a
handler is E5001 — there is no scheduler to yield to *from an interrupt*;
the suspension would never resume. Lab 11.2: a lock inside a lock is
E5002 — chapter 5's "forbids accumulate" arriving at its structural
consequence, and the lock-ordering grief (A then B here, B then A there)
refused before it exists.

The handler's *stack* has its own discipline, and its own budget. An
interrupt steals whatever stack the interrupted code left behind — in
Tyu the ISR context gets its own region precisely so its usage can be
measured separately — and the binding site carries a ceiling for it.
Chapter 6's E5040 (the self-recursive handler) was that ceiling refusing
infinity. Lab 11.5 is the finite version: thirty-four plates at peak,
against a budget of thirty-two:

```console
$ langc --emit=ir labs/ch11/red-04-deep-isr.mod --sysroot=sysroot
error[E5030]: typecheck error
```

(The production `tyu build` currently surfaces this rejection as a
different code — a masking quirk recorded in the book's notes — so the
lab uses the inspection path, where the checker's verdict is visible.
The verdict itself is unconditional: the handler's peak must fit the
budget.)

The unbounded case deserves its own sentence: a handler whose shape is
`⊤` — say, self-recursion — exceeds every ceiling, and E5040 refuses it
at the binding. And the budgets themselves are *binding-site* facts:
different vectors can carry different ceilings, set where the handler
meets the hardware. That is chapter 6's "tighter context, smaller shapes"
with real numbers attached.

## 11.5 The runtime boundary, stated

The compile-time proof is complete and verified. The *runtime*
demonstration — an actual timer interrupt firing on an actual emulated
board, the counter incremented from both contexts, atomicity asserted
end-to-end — exists as a fixture in the toolchain (`isr_lock_atomicity`),
targets the ARM board's SysTick timer, and is gated opt-in with the
tree's own comment: the interrupt delivery needs debugging; the
assertion currently loses the race against the test timeout.

The book's contract says to show the edge, so: **the daring part's
compile-time half is done and demonstrated; its end-to-end runtime half
is the known open edge**, exactly like chapter 6's native stack. The
compliant shape is proven; the hardware interrupt that would exercise it
is the workstream. Nothing in this chapter's claims exceeds that.

One more rule belongs here even though its enforcement lives in chapter
13: **dynamically loaded modules may not install interrupt handlers** (the
static-ISR rule). The reason is this chapter's rule itself. The
cross-context check is a whole-program property — it needs to see every
reacher of a resource at once. A module that could add an ISR at load
time would add reachability *after* the proof closed. Static vectors keep
the proof closed; swappable handler *logic* remains possible through a
statically declared trampoline word calling into loaded code. Remember
the shape of that trade when the loader's doors open.

---

## Labs — Chapter 11

### Lab 11.1 — The compliant shape *(green, build)*

```tyu
# labs/ch11/green-01-compliant-share.mod — Lab 11.1 (green)
# The compliant shape: the ISR and mainline both touch Counter, both
# inside their own lock. The cross-context rule is satisfied, and the
# lock lowers to whatever the platform needs (hosted: a fence).
# Expected: builds clean on hosted. The image itself is not runnable:
# an ISR binding is metal (chapter 8's QEMU track and real boards) - the
# compile-time binding checks are the lab.
module Compliant;
import platform/linux { platform.io.log };

resource Counter : i64;

@interrupt(TIMER0) : isr ( -- )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
;

: main ( -- i64 )
  Counter lock [ &!Counter @i64 drop ]
  "S\n" platform.io.log 0 ;
export { main };
end;
```

```console
$ tyu build labs/ch11/green-01-compliant-share.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch11-1
```

Build clean is the lab — and its quietness is the proof: the handler's
binding, its budget, its lock, and main's lock all passed. (The hosted
image is not runnable — an ISR binding is metal — so this green lab ends
at the build, like chapter 6's boundary lab ended at the OS.)

### Lab 11.2 — A lock inside a lock *(red)*

```tyu
# labs/ch11/red-01-nested-lock.mod — Lab 11.2 (red)
# A lock inside a lock: grants accumulate, forbids accumulate, and the
# nesting itself is rejected. Expected: E5002.
module NestTest;

resource R : i64;

@interrupt(TIMER0) : isr ( -- )
  R lock [ lock [ ] ]
;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** chapter 5's structural consequence, now in the ISR chapter where
it matters most: nested locks are the lock-ordering grief pattern, and
"no nesting" is what "forbids accumulate" produces when the forbidden
thing is re-entrant capture.

### Lab 11.3 — Yielding from an interrupt *(red)*

```tyu
# labs/ch11/red-02-isr-yield.mod — Lab 11.3 (red)
# An ISR forbids suspend: there is no scheduler to yield to from an
# interrupt. Expected: rejected at compile time — E5001.
module IsrYield;
import platform/linux { platform.task.yield };

@interrupt(TIMER0) : isr ( -- )
  platform.task.yield
;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** the ISR context forbids `suspend`. A yield from an interrupt
would suspend a context that nothing will ever resume — the scheduler was
interrupted, and it cannot be the resumer.

### Lab 11.4 — The race that does not ship *(red)*

```tyu
# labs/ch11/red-03-shared-unlocked.mod — Lab 11.4 (red)
# The cross-context rule: Counter is reachable from the ISR, so every
# access anywhere must be inside a lock - main's unlocked borrow is the
# race the rule exists to prevent. Expected: E5031.
module SharedUnlocked;

resource Counter : i64;

@interrupt(TIMER0) : isr ( -- )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
;

: main ( -- i64 )
  &!Counter @i64 drop
  0 ;
export { main };
end;
```

**Why:** one unlocked `@i64` in main, and the whole program is refused —
because the resource is interrupt-reachable, the unlocked read is the
lost-update race, and the compiler is the only reviewer who reads every
context. Fix it by locking main's access (that is Lab 11.1), or by
removing the ISR's reach — and confirm the rule tracks the latter, not
the ceremony.

### Lab 11.5 — Thirty-four plates in a thirty-two-plate room *(red, inspection check)*

```tyu
# labs/ch11/red-04-deep-isr.mod — Lab 11.5 (red, inspection check)
# The ISR's own stack has a budget; 34 plates at peak exceeds it. Check
# with: langc --emit=ir red-04-deep-isr.mod --sysroot=<sysroot>
# Expected: rejected — E5030 (ISR stack exceeds its budget).
module Main;
@interrupt(TIMER0) : isr ( -- )
  0
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop
;
end;

```

```console
$ langc --emit=ir labs/ch11/red-04-deep-isr.mod --sysroot=sysroot
error[E5030]: typecheck error
```

**Why:** the handler's stack peaks at thirty-four plates against a
thirty-two-plate budget, and the binding-site check refuses it. Tune the
body down to fit the budget and it compiles — the budget is a number, and
meeting it is the handler author's job. (This lab's rejection is checked
with the inspection path; see the chapter's note on the production
pipeline's masking quirk.)

### Post-mortem — the lost update *(optional — for readers with C or embedded scars)*

The canonical C incident: a multi-byte counter or a flag bundle touched by
a timer ISR and by mainline, "protected" by disabling interrupts on one
path but not the other three, or protected everywhere except the one place
added last month. The desktop build passes; the field unit hangs or loses
counts at 2 percent; the fix is a two-line change that cannot be
regression-tested because nobody could reproduce the interleaving.

The cross-context rule replaces that whole genre with one question the
compiler answers before the program exists: *which contexts reach this
storage, and is every one of them inside the correct lock?* The unlocked
straggler is E5031 — not a code review comment, not a race to reproduce,
but a rejected program. And the nesting ban (E5002) closes the adjacent
graveyard where the fix for one race was a second lock and the fix for
that was a deadlock.

### From the workbench

Why can a dynamically loaded module not install an ISR? Because this
chapter's central check is whole-program: it must see every reacher of a
resource at once, and a runtime-installed handler is a reacher that
appears after the proof has closed. The static-ISR rule is not a
limitation bolted onto the loader — it is what makes the cross-context
rule *decidable*, and therefore what makes the field-update story of
chapter 13 safe at all. Loaded code computes under the interrupt regime
the static program proved; it cannot change the regime. Every check in
this book has been buying exactly this: the next chapter's daring is only
daring because the last chapter's proof closed first.

### Exercises

1. **Both directions of the rule.** Take Lab 11.4 and fix it two ways:
   first by locking main's access; then by deleting the ISR's access to
   `Counter` entirely (leaving main unlocked). Confirm both compile, and
   explain why the second fix is legitimate — the rule tracks
   *reachability*, not ceremony. Which fix would you ship, and why?
2. **Choose the lowering.** For each platform, name the lowering from
   §11.2's table: a resource touched only by main; a resource touched by
   main and a SysTick handler on a single-core MCU; the same on a
   dual-core part. Which capability from chapter 5 does the dual-core
   case require, and where is it declared?
3. **Close the doors, prove the house.** In two sentences, explain to a
   colleague why the static-ISR rule exists, using the words
   *whole-program*, *reachability*, and *closed*. Then predict what a
   dynamically installed handler would do to the E5031 check.

---

*Next: chapter 12 — Tasks and Channels: Concurrency the Platform
Provides. The scheduler, the yield, the channel as custody at a distance —
and why the concurrency chapter of a systems book is short when the
effects are already declared.*
