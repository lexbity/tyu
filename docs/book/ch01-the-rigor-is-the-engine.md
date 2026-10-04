# Chapter 1 — The Rigor Is the Engine

> A program is a stack of promises.
> The only question is who checks them: the author, at 3 a.m. —
> or the machine, before lunch.
>
> — workshop wall

## 1.1 Two ways to be careful

There are two ways to write careful software.

The first is **vigilance**. The ritual is familiar. A function gets a
precondition that isn't expressible in the type system, so the author writes
it down — in a comment, or a naming convention, or a README that everyone is
*sure* to read. For a while, everyone who calls the function has read the
comment. The code works. Then a new engineer joins, or a call site is copied
under deadline, or the comment is updated on one side of a refactor and not
the other. Nothing rejects the lie. The program ships. The lie ships with it,
and the lie becomes a field failure — at a customer site, at 3 a.m., with a
debugger that had to be driven in from nine hours away.

Vigilance is a real skill, and for decades it was the *only* skill available.
But it has a property that makes it dangerous as a foundation: **it does not
compose**. A system maintained by care is only as careful as its most recent
editor. The discipline lives in people, and people get tired, and people
leave.

The second way is **verification**: move the promises out of comments and
into the program, where something mechanical can check them — before the
program runs, or at the exact moment a promise is broken. The discipline then
lives in the code, where it survives refactors, handoffs, and midnight
copy-paste.

This book is about the second way, practiced in a language designed for it.

**Tyu** is a small, statically typed, *concatenative* systems language:
programs are built from **words** that transform a **data stack**, the way
Forth programmers have composed programs for fifty years. What Tyu adds is a
verification habit borrowed from Ada, SPARK, and Rust: nearly everything
written down about a word — what it takes, what it returns, what it may *do*,
what it *promises* — is a machine-checked claim, not a comment.

Here is the thesis of this whole book, stated once and then demonstrated for
fourteen chapters:

> **Safety is not the brake — it's the engine. The rigor is what licenses the
> daring parts.**

The daring parts are coming. Chapter 11 shares a hardware device between an
interrupt handler and mainline code, with proof instead of prayer. Chapter 13
loads signed code into a running system from untrusted media. Nobody sane
attempts those in an unverified language. Nobody has to attempt them
unverified again.

But daring is earned in order. First, the machine has to start telling the
truth. That starts now.

## 1.2 The first ten minutes

This book assumes almost nothing: the Tyu toolchain is installed (`tyu` and
`langc` — see Appendix C), and there is a computer. Every lab in every
chapter runs there, on the hosted Linux target. No boards, no simulators, no
special hardware — that comes later, and only where the *concept* demands it.

The convention throughout: labs run from the root of the book's repository,
where they live under `devdocs/book_v3/labs/`. Here is the first program:

```tyu
# labs/ch01/green-01-hello.mod — Lab 1.1 (green)
# Expected: prints one line, emits the completion marker, exits 0.
module Hello;
import platform/linux { platform.io.log };

: main ( -- i64 )
  "assembling in tyu\n" platform.io.log
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

Start with the `:` — read it as *define a word*. This program defines a word
named `main`, and after its name comes this:

```tyu
( -- i64 )
```

That is a **stack effect**: a claim about the data stack. Its meaning, spoken
by the word itself: *"run me on any stack; when I am done, one signed 64-bit
integer will sit where nothing sat before."* In Tyu this is not documentation
— it is the first contract in the book, and the checker reads it. The hosted
runtime uses `main`'s returned value as the process exit code, so `main` ends
by pushing `0`.

The body reads in execution order, left to right, the way the machine will
run it:

- `"assembling in tyu\n"` pushes a string onto the data stack;
- `platform.io.log` consumes it and prints it;
- `"S\n"` and another `platform.io.log` emit two bytes, `S` and newline;
- `0` pushes the exit code.

That `S` line is not decoration. Tyu's runner and test harness listen for the
**completion marker**: a program that exits *without* emitting `S\n` has not
finished its work, whatever its exit code claims. A program dies exactly that
way in a moment — and the marker's absence is the proof.

Build it and run it two ways:

```console
$ tyu build devdocs/book_v3/labs/ch01/green-01-hello.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/green-01
$ ./build/green-01/image.elf
assembling in tyu
S
$ echo $?
0
```

The direct run shows the program's own output. The driver's run adds the
harness point of view — it consumes the program's output, checks the marker,
and renders a verdict:

```console
$ tyu run devdocs/book_v3/labs/ch01/green-01-hello.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/green-01
tyu: resolved profile '(implicit all-features-on)' → features: [concurrency, module-loading]
    …
$ echo $?
0
```

Silence, from the harness, is success. Hold that thought.

## 1.3 The machine that tells the truth

Now a word that makes a promise. This is Lab 1.2:

```tyu
# labs/ch01/green-02-promise-kept.mod — Lab 1.2 (green)
# Expected: two "level accepted" lines, the completion marker, exit 0.
module Level;
import platform/linux { platform.io.log };

: set-level ( i64 -- )
  needs [ dup 0 >= ]
  => level
  "level accepted\n" platform.io.log ;

: main ( -- i64 )
  42 set-level
  100 set-level
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

`set-level` takes one integer and, before its body runs anything, evaluates
its **precondition** — the `needs` clause. Read the clause as a tiny program
on the incoming stack: `dup` copies the input, `0` pushes zero, `>=` replaces
the pair with a boolean. The clause's own shape is checked: a precondition
must end with *exactly one* boolean left on top. (Its postcondition twin,
`ensures`, runs after the body; chapter 4 treats it properly.)

If the precondition holds — if the level is zero or more — the body runs, and
`=> level` names the input so the rest of the word can read it. Run it:

```console
$ ./build/green-02/image.elf
level accepted
level accepted
S
$ echo $?
0
```

Two calls, two promises kept, marker emitted, exit zero. Now break the
promise. This is the chapter's engine demo — the same program, except `main`
calls `set-level` with `-5`:

```tyu
# labs/ch01/engine-broken-promise.mod — Lab 1.3 (engine)
# Expected: compiles cleanly; traps at run time with CONTRACT_FAIL
# (trap code 20, hosted exit status 20); never emits the marker.
module BrokenPromise;
import platform/linux { platform.io.log };

: set-level ( i64 -- )
  needs [ dup 0 >= ]
  => level
  "level accepted\n" platform.io.log ;

: main ( -- i64 )
  42 set-level
  -5 set-level
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

`needs [ dup 0 >= ]` is a claim about every possible caller, forever. The
compiler reads it and inserts a check at the front of `set-level` — the check
lives *in the callee*, so every caller, present and future, is held to it.
Run it, first from the harness's point of view:

```console
$ tyu run devdocs/book_v3/labs/ch01/engine-broken-promise.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/engine
tyu: resolved profile '(implicit all-features-on)' → features: [concurrency, module-loading]
    …
tyu: NO_COMPLETION — exited with code 20 but no `S\n` marker
$ echo $?
1
```

And from the program's own:

```console
$ ./build/engine/image.elf
level accepted
$ echo $?
20
```

Read that pair of facts, because they are the whole chapter.

The first call succeeded — `level accepted` printed once. Then `-5` arrived,
the inserted check evaluated `-5 ≥ 0` to false, and the runtime **trapped**.
The process ended with status **20**, and the completion marker — the `S`
that proves the program did what it set out to do — never came. The harness
translated the missing marker into `NO_COMPLETION`. The two lines after the
lie never ran; their output never exists.

Tyu's runtime failures speak in small, named integers — the **trap codes**:

| Code | Name             | Meaning                                   |
|------|------------------|-------------------------------------------|
| 10   | `STACK_OVERFLOW` | the data stack exceeded its bound          |
| 20   | `CONTRACT_FAIL`  | a `needs`/`ensures` promise was broken     |
| 21   | `SUBTYPE_FAIL`   | a value left its declared range (ch. 3)    |
| 22   | `ASSERT_FAIL`    | an assertion failed                        |
| 23   | `UNREACHABLE`    | control reached a state declared impossible|

Exit code 20 is not a crash. It is a *verdict*: "a promise was broken, here
is which class of promise, and here is where the program stopped." Compare
this with the vigilance model, where the same bug is a negative level flowing
quietly into code written by someone who "just knew" levels are non-negative.
The lie was caught at the boundary, early, loudly, and cheaply — and that is
exactly the point. **Checked failures are the cheapest failures a program can
have.**

## 1.4 The Practice

Every chapter of this book opens with its practice — the one checkable
discipline the chapter adds. The practice of chapter 1 is the practice of the
whole book, in miniature:

> **The Practice.** *Everything claimed, the machine checks. Claims are
> cheap; lies are loud.*

Three species of claim have already appeared, in twenty minutes:

| The claim                                       | Who checks it        | Where |
|-------------------------------------------------|----------------------|-------|
| "my inputs and outputs have these types"        | the type checker     | ch. 2 |
| "my values stay inside these ranges"            | the subtype system   | ch. 3 |
| "such-and-such is true whenever I run"          | contracts            | ch. 4 |
| "I only ever do *these* things"                 | effects & capabilities | ch. 5 |
| "my stack never grows past *this*"              | the stack-bound analyzer | ch. 6 |
| "nobody else writes while I hold this"          | the borrow checker   | ch. 7 |
| "I only touch this hardware *this way*"         | MMIO types           | ch. 8 |
| "this device is safe to share with the ISR"     | the cross-context rule | ch. 11 |
| "this module is authentic and ABI-compatible"   | the loader           | ch. 13 |

The rest of the book is the craft of making claims that hold — and of
discovering, over and over, that the claims are what license the daring.

## 1.5 What this book promises — and what it doesn't

Contracts in Tyu are checked **at run time**, as just seen. A future
milestone of the project — with tooling to match — is *static discharge*:
proving preconditions before the program ever runs, SPARK-style. This book
teaches contracts the way the prover will want them: small, side-effect-free,
arithmetic. That discipline costs nothing today, and it is the same
discipline the prover will check tomorrow. When the tooling lands, the
chapter 4 habits will already be proofs in waiting.

And: this book teaches Tyu **as it is**. Every green lab compiles and runs on
the hosted target; every red lab is rejected, with the exact error code
printed in the text; every runtime-failure lab traps with the exact trap
code. All of it is executed in the project's CI, so the book cannot silently
drift from the compiler. Where the language has edges — and it does — the
edges appear in the text.

## 1.6 How to read this book

Each chapter is a discipline, and each discipline comes with three kinds of
lab:

- **Green labs** compile and run. The transcript in the text is real.
- **Red labs** must *not* compile. The task: make the rejection happen, quote
  the error code, and name the falsified claim. (The reasoning follows each
  lab as a *Why* note.)
- **Post-mortems** are the anti-patterns that kill real projects — shown
  first the way they kill in C, then the way Tyu catches them.

And in each chapter, a short **From the workbench** note: why the rule is the
way it is, from the people who made it.

---

## Labs — Chapter 1

### Lab 1.1 — First words *(green)*

```tyu
# labs/ch01/green-01-hello.mod — Lab 1.1 (green)
# Expected: prints one line, emits the completion marker, exits 0.
module Hello;
import platform/linux { platform.io.log };

: main ( -- i64 )
  "assembling in tyu\n" platform.io.log
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

Build it, run it both ways (§1.2), and confirm the marker and exit code 0.

**Notice:** the stack effect `( -- i64 )` on `main` is checked like any
other. Try to fool it — that's Red lab 1.4.

### Lab 1.2 — A promise kept *(green)*

```tyu
# labs/ch01/green-02-promise-kept.mod — Lab 1.2 (green)
# Expected: two "level accepted" lines, the completion marker, exit 0.
module Level;
import platform/linux { platform.io.log };

: set-level ( i64 -- )
  needs [ dup 0 >= ]
  => level
  "level accepted\n" platform.io.log ;

: main ( -- i64 )
  42 set-level
  100 set-level
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

Run it, then answer before running: *how many times* will `level accepted`
print, and why does the answer depend on the `needs` clause and not the body?

### Lab 1.3 — The broken promise *(engine)*

```tyu
# labs/ch01/engine-broken-promise.mod — Lab 1.3 (engine)
# Expected: compiles cleanly; traps at run time with CONTRACT_FAIL
# (trap code 20, hosted exit status 20); never emits the marker.
module BrokenPromise;
import platform/linux { platform.io.log };

: set-level ( i64 -- )
  needs [ dup 0 >= ]
  => level
  "level accepted\n" platform.io.log ;

: main ( -- i64 )
  42 set-level
  -5 set-level
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

Run it both ways (§1.3). Confirm three facts: one `level accepted`; exit
code 20; no `S` marker. Then swap the two calls in `main` (`-5` first, `42`
second), write down the predicted transcript *before* running, and compare.

### Lab 1.4 — `main` forgets its promise *(red)*

```tyu
# labs/ch01/red-01-main-no-exit.mod — Lab 1.4 (red)
# Expected: rejected at compile time — E1018 (main must return the exit code).
module NoExit;
import platform/linux { platform.io.log };

: main ( -- )
  "S\n" platform.io.log ;

export { main };
end;
```

This `main` declares `( -- )` — no exit code. Build it; it must be rejected.
Expected:

```console
error[E1018]: for --emit=obj, main must return exactly one value (exit code)
```

**Why:** on the hosted target the exit code is `main`'s output value. A
`main` that promises no output has broken a claim the *platform* makes, and
the compiler holds every program to the platform's contract, not just the
program's own.

### Lab 1.5 — A word lies about the stack *(red)*

```tyu
# labs/ch01/red-02-stack-lie.mod — Lab 1.5 (red)
# Expected: rejected at compile time — E3220 (declared stack effect vs body).
module StackLie;

: answer ( -- i64 ) ;

: main ( -- i64 )
  answer
  "S\n" platform.io.log
  0 ;

import platform/linux { platform.io.log };
export { main };
end;
```

The word `answer` claims `( -- i64 )` — "one integer out" — but its body is
empty: no integer is produced. Build it; it must be rejected with **E3220**,
the checker's declared-versus-computed stack mismatch.

**Why:** in most languages, the signature is a promise with no enforcement.
In Tyu the signature is a constraint. The checker ran `answer`'s body against
its claim and found the claim one value short.

### Lab 1.6 — A word that does not exist *(red)*

```tyu
# labs/ch01/red-03-unknown-word.mod — Lab 1.6 (red)
# Expected: rejected at compile time — E3210 (WordNotFound).
module Unknown;

: warmth ( i64 -- i64 )
  1 + ;

: main ( -- i64 )
  41 OOPS
  warmth
  "S\n" platform.io.log
  0 ;

import platform/linux { platform.io.log };
export { main };
end;
```

`main` calls `OOPS`, which is defined nowhere. Rejection: **E3210**
(`WordNotFound`).

**Why:** the checker knows every word in scope — builtins, imports,
definitions. There is no "linker will find it" escape hatch, which means a
whole class of runtime surprises becomes a compile-time sentence.

### Post-mortem — the comment that lied

Here is the C that this chapter is about. Every embedded team has a version
of it:

```c
/* set_level: level must be >= 0. */
void set_level(int level) { ... }

/* 400 calls later, in a different file, by a different hand: */
set_level(-1);   /* compiles fine. ships fine. */
```

The comment stated a true fact about the system, and the system accepted a
false one, and nothing in the toolchain was allowed to notice the difference.
Projects do not usually die of one such lie; they die of the *drift* — the
comment updated without the call sites, the call sites written without the
comment, until the documentation describes a program that no longer exists.

Tyu's answer is not "comments are bad." It is: *if a fact is load-bearing, it
is not a comment.* `needs [ dup 0 >= ]` is the same sentence as the C
comment, but the compiler reads it, the runtime enforces it, and the cost of
ignoring it moved from 3 a.m. to build time — or, for claims only the running
program can evaluate, to the exact boundary where the lie would have entered.

### From the workbench

A question worth asking: *why is `needs` a separate clause, instead of
writing the check as the first thing in the body?*

Because the difference is who can see it. A check written in the body is an
implementation detail — it protects that word, today. A `needs` clause is
part of the word's *interface*: it is stored with the word, mirrored in the
module's interface file, hashed into the module's ABI identity, and enforced
once for every caller that will ever exist. The clause turns a habit into a
property.

### Exercises

1. **Predict, then run.** In Lab 1.3's swapped-call variant, exactly one
   `level accepted` prints. Explain why the second call's body — the string,
   the log, the marker — never runs, using the words *precondition*, *trap*,
   and *contract*.
2. **Make a red lab of your own.** Write a word whose body produces *two*
   values but claims one. Confirm the checker rejects it with E3220, and
   write one sentence naming the falsified claim.
3. **The reported result.** Change `main` in green-01 to return `1` instead
   of `0`, run it with `tyu run`, and explain the verdict — the program
   *finished* (its marker came out), yet the harness still objects. Which
   layer noticed which fact?

---

*Next: chapter 2 — First Words. The data stack, for real this time: reading
stack effects fluently, naming values with `=>`, and the three control words
(`if`, `while`, `loop`) that turn a straight line into a program.*
