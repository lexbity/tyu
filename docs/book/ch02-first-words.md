# Chapter 2 — First Words

> Data stack, data stack, does whatever a data stack does.
> Push a thing, push a thing, then a word consumes the two.
>
> — workshop wall, after a hard day

## 2.1 The Practice

> **The Practice.** *Every word declares its effect on the stack, and the
> checker holds the body to the declaration — one term at a time, left to
> right, exactly the way the machine runs them.*

Reading a Tyu program and running a Tyu program are the same activity,
performed at different speeds. This chapter builds that one skill — after
building the two ideas underneath it: the stack, and the word.

## 2.2 The data stack

Picture a stack of dinner plates.

A fresh plate goes on top. A plate comes off from the top. Nobody — not the
dishwasher, not the chef — reaches in and pulls the third plate from the
bottom. The stack has exactly one door, and the door is at the top. The
first plate placed will be the last plate taken: the stack remembers order,
and it remembers it ruthlessly.

That is the entire machine. A Tyu program runs on a **data stack** that
behaves just so:

- **Push** — put a value on top. A number literal does this: writing `3`
  in a program means "put the number three on the stack."
- **Take and transform** — a word takes some values off the top, does its
  work, and puts its results back on the top.

There is nothing else. No hidden shelves, no side tables, no bag of state a
word can rummage through when no one is looking. At any instant, the whole
world of a running Tyu program — the part that concerns data — is *the
stack*, and the top of it is where all the action is.

Why build a language on such a spartan piece of furniture? Three reasons,
and they compound.

**Order becomes visible.** `2 3 +` pushes two, pushes three, then adds the
top two — three on top of two. `3 2 +` is a different sentence with the same
answer and a different meaning where subtraction is involved: `10 3 -` is
seven, `3 10 -` is negative seven. The stack records the order the program
wrote, left to right, the same way the machine will run it.

**Every word has a small, fully visible interface.** Because the stack is
the *only* way data moves between words, a word's entire relationship with
the rest of the program fits in one line: what it takes, and what it leaves.
That line is called the **stack effect**, and because it is complete, it can
be *checked* — the foundation of everything this book does.

**The program can be replayed.** A program with hidden state must be
understood by running it. A stack program can be *simulated with a pencil*:
write down the stack, read the words left to right, update the paper. The
checker does exactly this simulation before the program is allowed to exist —
and §2.4 shows the tool that prints its homework.

Values on the stack are typed. This chapter lives on two types: `i64`, the
signed 64-bit integer — the everyday counting number — and `bool`, which is
`true` or `false` and nothing else. Chapter 3 adds many more; these two
carry the whole chapter.

## 2.3 Words: the vocabulary of the language

A **word** is a named transformation of the stack. That is the whole
definition, and it is worth slowing down to see why such a small idea can
carry a language.

`+` is a word: it takes two numbers off the top and leaves their sum. `dup`
is a word: it takes the top value and leaves two of it. `check` — from
chapter 1 — is a word. `main` is a word. Even the program itself is, in the
end, just a word called `main` that the runtime calls. The language has two
kinds of steps and nothing more: **literals**, which put a value on the
stack, and **words**, which transform it.

Words matter because they are the *only* way to make the language bigger.
A word has a name (so it can be used again), a stack effect (so every use
can be checked), and a body (which is itself just more words — words calling
words). Define a word and it becomes indistinguishable from the built-ins:
nothing anywhere in the language can tell `check` was written by a
programmer on a Tuesday and `drop` was built into the compiler. Chapter 2.8
turns that fact into a working method: the standard library is not a
cabinet the language was shipped with — it is Tyu source, and when a word is
missing, the fix is to write it.

Programs read like sentences because words *are* a vocabulary. The built-in
catalog is small enough to hold in the head — the words used in this chapter
are:

| Word | Stack effect      | Meaning                          |
|------|-------------------|----------------------------------|
| `dup` | `( i64 -- i64 i64 )` | copy the top value           |
| `drop` | `( i64 -- )`        | discard the top value        |
| `swap` | `( i64 i64 -- i64 i64 )` | exchange the top two    |
| `==` `<` `>=` … | `( i64 i64 -- bool )` | compare the top two |
| `and` `or` | `( bool bool -- bool )` | combine two conditions |
| `not` | `( bool -- bool )`    | negate a condition      |

The complete catalog of built-in words, with their full declarations, is
table A.1 in Appendix A — worth a skim now and a reference forever. The
notation `( before -- after )` is the **stack effect**: the stack before the
word runs, left of the `--`; the stack after, right of it; the top of the
stack is always listed rightmost. `dup` takes a world whose top is an `i64`
and returns a world whose top is an `i64` sitting on another `i64`.

## 2.4 Reading = running

When the checker rejects a program, its message is terse on purpose — one
line, one error code. The tool that *shows its work* is the typecheck trace.
Run it on a program and it prints the stack state after every single term,
which is precisely the pencil simulation of §2.2, performed by the machine:

```console
$ langc --emit=tc devdocs/book_v3/labs/ch02/green-01-arithmetic-check.mod \
        --sysroot=sysroot
word main (  -- i64 )
  2 | stack: i64
  3 | stack: i64 i64
  + | stack: i64
  5 | stack: i64 i64
  == | stack: bool
  check | stack:
  10 | stack: i64
  3 | stack: i64 i64
  - | stack: i64
  …
```

Left to right, one term per line, stack contents after each. When a red lab
rejects with `E32xx` and does not name the word — E3220, met below, is
notoriously quiet — `--emit=tc` names it, line by line. Keep the habit: **on
any typecheck error, read the trace before touching the code.**

## 2.5 Naming values

Four coordinates go into a distance-squared calculation: `x1 y1 x2 y2`. On a
stack, they arrive deepest-first, and computing `x2 x1 -` means reaching past
two values to find them. It can be done with shuffles. It should not be.

Tyu's answer is the binding word `=>`:

```tyu
: dist2 ( i64 i64 i64 i64 -- i64 )
  => y2 => x2 => y1 => x1
  x2 x1 - dup *
  y2 y1 - dup * + ;
```

Each `=> name` pops the top value and gives it a name, scoped to the word.
The four bindings empty the stack; the body then reads like the formula it
is. Naming is not a convenience bolted onto the language — it is how the
stack effect stays truthful at a glance: four in, the bindings account for
all four, one out.

A rule of thumb with the force of style: **beyond two or three stack items,
name them.** The deep-juggling style that old Forth code was famous for is a
disease Tyu makes easy to avoid — and chapter 9 turns that style rule into a
proof obligation.

## 2.6 Decisions: control flow and `if`

Every program so far has been a straight line: the same words run in the
same order every time, from `main` to the marker. The moment software stops
being a demo and starts being software, it must **choose** — lock the door
if it is after midnight, otherwise leave it; retry if the sensor answered,
give up if it did not. Programming calls this *control flow*: the ability of
a program to run different words under different conditions instead of
marching through one fixed list.

Choice needs three ingredients:

1. **A question with a yes-or-no answer** — in Tyu, a `bool` on the stack,
   usually from a comparison word (`<`, `>=`, `==`). This is called the
   **condition**.
2. **Two plans of action** — the words to run if the answer is `true`, and
   the words to run if it is `false`.
3. **A way to pick** — a word that takes the question and the two plans and
   runs exactly one.

The picker is `if`, and the two plans are written in square brackets:

```tyu
condition [ words-if-true ] [ words-if-false ] if
```

The brackets are themselves worth a pause. `[ … ]` is a **quotation**: a
piece of program sitting *on the stack as a value*. Nothing in it runs when
it is pushed — it is a plan, written down and waiting. `if` is the word that
executes one plan: it takes the boolean, takes the two quotations, runs one,
and discards the other, unopened.

Now chapter 1's `check` word can be read in full:

```tyu
: check ( bool -- )
  not [ "F\n" platform.io.log ] [ ] if ;
```

`check` takes one boolean — the result of some test. `not` flips it, so from
here on, `true` means "the test failed." `if` receives the flipped boolean
and two plans: if the test failed, print `F\n` — the harness's failure
marker; if it passed, do nothing (the empty quotation). One word, one
decision, and every test in this book since chapter 1 has been a call to it.

`if` is an *expression* as well as a decision: both quotations must leave
the stack in the same state, and whatever they leave is what `if` leaves.
That rule does real work twice in one line of Lab 2.3:

```tyu
10 count-up 10 == [ "S\n" ] [ "F\n" ] if platform.io.log
```

Both plans produce a `str`, so `if` produces a `str`, which
`platform.io.log` consumes. Had one branch left a string and the other an
integer, the checker would reject the word at compile time — Lab 2.7 makes
that rejection happen.

## 2.7 Repetition: loops and `while`

The second idea beyond the straight line is **repetition**. Counting to ten
by writing `1 + 1 + 1 +` ten times is not programming; software exists to
run the same words as many times as the *data* demands — once for every
sample, every pixel, every packet. A **loop** is a plan of action (a body)
plus a question (a condition), run again and again until the answer says
stop.

Tyu's loop is `while`, and its shape states the contract outright:

```tyu
[ condition ] [ body ] while
```

The machine plays a simple game: ask the *condition* quotation — is it
`true`? If yes, run the *body* quotation once, then go back and ask again.
The first time the condition comes up `false`, the loop ends and the program
continues from the word after `while`. Each lap re-asks the question, so
something inside the body had better move the world toward "no" — or the
question never changes and the loop never ends.

That last danger is exactly why `while`'s checking rules are strict. The
condition must run on the *same* stack the loop started with — it is a
question, not a renovation — and the body must be **net-zero**: consume
exactly what it produces. A body that pushed one extra value per lap would
pile plates forever and eventually eat the machine; in Tyu, `E3257`/`E3258`
stop it at compile time, before there is a machine to eat.

The counter therefore lives *in* the loop, on the stack, where the net-zero
body consumes and restores it each pass:

```tyu
: count-up ( i64 -- i64 )
  => limit
  0
  [ dup limit < ] [ 1 + ] while ;
```

Bind the limit into a local, push `0` as the counter, then loop. Read the
condition as the pencil simulation: `dup` copies the counter (the question
needs to look at it without destroying it), `limit <` asks "is the counter
still below the limit?" — and leaves the counter right where it was, one
boolean richer, which `while` consumes. The body adds one to the counter and
leaves exactly the counter. Net-zero, lap after lap, until the question says
stop — and `count-up`'s result is the limit itself.

## 2.8 Writing a word the language is missing

The builtin table is tiny on purpose — three shuffles, comparisons, logic.
Everything else in the vocabulary is written in Tyu, including the whole
sysroot. That has a consequence worth more than its size suggests: **when a
word is missing, the fix is to write the word.**

`over` is the most-missed shuffle — `( i64 i64 -- i64 i64 i64 )`, a copy of
the *second* item over the top. The shuffles that ship cannot build it; `dup`
copies only the top. But §2.5's naming rule builds it in one line:

```tyu
: over ( i64 i64 -- i64 i64 i64 )
  => b => a
  a b a ;
```

Name the two inputs, then push them back in order — with the first one
repeated. The declaration is checked like any other, and the definition is
now vocabulary: every word below it in the file may use `over` as if it had
always existed. Lab 2.4 builds it and tests it.

This is the first taste of a theme the book returns to in chapter 9 and
chapter 10: in Tyu, the language and the library are made of the same thing,
and the boundary moves with the problem.

---

## Labs — Chapter 2

### Lab 2.1 — Arithmetic under check *(green)*

```tyu
# labs/ch02/green-01-arithmetic-check.mod — Lab 2.1 (green)
# Expected: silent clean run (exit 0, marker emitted, no F).
module ArithmeticCheck;
import platform/linux { platform.io.log };

: check ( bool -- )
  not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  2 3 + 5 == check
  10 3 - 7 == check
  4 5 * 20 == check
  3 5 < check
  7 2 > check
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

```console
$ tyu run devdocs/book_v3/labs/ch02/green-01-arithmetic-check.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/g2-1
tyu: resolved profile '(implicit all-features-on)' → features: [concurrency, module-loading]
    …
$ echo $?
0
```

Silence is success. Then sabotage one comparison, rebuild, and meet the
other verdict:

```console
$ tyu run devdocs/book_v3/labs/ch02/green-01-arithmetic-check.mod …
tyu: FAIL_MARKER — 1 failure(s) reported
$ echo $?
2
```

The `F` marker is not a crash: the program *ran to completion* and reported
which of its claims failed. Predict which of the five checks died, then
confirm. (A failing-check variant of the lab was run for this transcript:
`tyu run` exits **2**, by its own classification — pass is 0, a trap is
`NO_COMPLETION`, a hang is 124.)

### Lab 2.2 — Naming the coordinates *(green)*

```tyu
# labs/ch02/green-02-locals-dist2.mod — Lab 2.2 (green)
# Expected: silent clean run (exit 0, marker emitted, no F).
module Dist2;
import platform/linux { platform.io.log };

: check ( bool -- )
  not [ "F\n" platform.io.log ] [ ] if ;

: dist2 ( i64 i64 i64 i64 -- i64 )
  => y2 => x2 => y1 => x1
  x2 x1 - dup *
  y2 y1 - dup * + ;

: main ( -- i64 )
  1 2 4 6 dist2 25 == check
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

Run it clean, then read its trace and follow along:

```console
$ langc --emit=tc devdocs/book_v3/labs/ch02/green-02-locals-dist2.mod \
        --target=x86_64-unknown-linux-gnu --sysroot=sysroot
word dist2 ( i64 i64 i64 i64 -- i64 )
  => | stack: i64 i64 i64
  => | stack: i64 i64
  => | stack: i64
  => | stack:
  x2 | stack: i64
  x1 | stack: i64 i64
  - | stack: i64
  dup | stack: i64 i64
  * | stack: i64
  …
```

**Notice:** the four `=>` lines *drain* the incoming stack to empty — the
bindings account for every declared input, right there in the trace.

### Lab 2.3 — A counted loop *(green)*

```tyu
# labs/ch02/green-03-while-counter.mod — Lab 2.3 (green)
# Expected: prints "S" (the count-up result equals 10), exits 0.
module CountUp;
import platform/linux { platform.io.log };

: count-up ( i64 -- i64 )
  => limit
  0
  [ dup limit < ] [ 1 + ] while ;

: main ( -- i64 )
  10 count-up 10 == [ "S\n" ] [ "F\n" ] if platform.io.log
  0 ;

export { main };
end;
```

Run it, then answer before modifying: what does `count-up` leave on the stack
when the limit is `0`? Predict, change `10` to `0` in `main`, run, and
explain using the words *net-zero* and *condition*.

### Lab 2.4 — Build the vocabulary: `over` *(green, build-the-vocabulary)*

```tyu
# labs/ch02/green-04-over-from-locals.mod — Lab 2.4 (green, build-the-vocabulary)
# The checker ships three shuffles (dup swap drop); `over` is missing.
# `over` is written here, in Tyu, out of locals — and tested.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Over;
import platform/linux { platform.io.log };

: check ( bool -- )
  not [ "F\n" platform.io.log ] [ ] if ;

: over ( i64 i64 -- i64 i64 i64 )
  => b => a
  a b a ;

: main ( -- i64 )
  1 2 over + + 4 == check        # 1 2 1  ->  1+2+1 = 4
  7 8 over swap drop == check    # 7 8 7  ->  after swap/drop: 7 7, equal
  "S\n" platform.io.log
  0 ;

export { main };
end;
```

Run it clean. Then write `2dup ( i64 i64 -- i64 i64 i64 i64 )` the same way,
add a `check` for it, and note what its stack effect *means* in words
("copy both").

### Lab 2.5 — Nothing to drop *(red)*

```tyu
# labs/ch02/red-01-underflow.mod — Lab 2.5 (red)
# Expected: rejected at compile time — E3202 (stack underflow: nothing to drop).
module Underflow;

: main ( -- i64 )
  drop
  0 ;

export { main };
end;
```

**Why:** `drop` claims `( i64 -- )` — it cannot run on an empty stack, and
the checker proves no value ever arrives. In most languages this class of bug
(eat a value that isn't there) is a runtime surprise; here it is a
compile-time sentence.

### Lab 2.6 — The forgotten return *(red)*

```tyu
# labs/ch02/red-02-forgotten-return.mod — Lab 2.6 (red)
# main calls twice, which leaves TWO values; the declaration promises ONE.
# Note: the error does not name the word; see §2.4 for the
# `langc --emit=tc` idiom that shows where the counts go wrong.
module ForgottenReturn;

: dice ( -- i64 )
  6 ;

: main ( -- i64 )
  dice
  dice
  "S\n" platform.io.log
  0 ;

import platform/linux { platform.io.log };
export { main };
end;
```

**Why:** each call to `dice` leaves a value. Two calls leave two; the string
and the marker and the `0` pile on top; `main` declared *one* output and ends
with three. Run `langc --emit=tc` on it and find the first line where the
counts stop matching the declaration — the trace names the word the error
message does not.

### Lab 2.7 — Branches that disagree *(red)*

```tyu
# labs/ch02/red-03-branch-mismatch.mod — Lab 2.7 (red)
# Expected: rejected at compile time — E3246 (branch depth mismatch).
module BranchMismatch;

: main ( -- i64 )
  true [ 0 ] [ 0 0 ] if drop
  0 ;

export { main };
end;
```

**Why:** `if` must know what it leaves behind, so both branches must end in
the *same* stack state — same depth, same types. One branch leaves one value,
the other leaves two, and no single prediction of the program's future stack
exists. The checker refuses to guess. (A condition that isn't a boolean is a
different rejection, E3243 — try `1 [ … ] [ … ] if` and compare.)

### Post-mortem — stack rot *(optional — for readers with C or embedded scars)*

The old Forth sin this chapter quietly outlaws: `over rot swap - rot! tuck`
spaghetti, where the third item's meaning depends on the line's *length*. The
C equivalent is the eight-argument function called positionally in forty
places — every call site a small puzzle of "which zero is `src_len` again?"
Both rot the same way: meaning that lives in position, invisible at the call
site, wrong after the first signature change.

The Tyu habit is `=>` at depth and words at width: name the items, then
write the formula. The compiler enforces the declaration at every call site,
so a signature change is not a puzzle but a list of compile errors — each
one a call site that was quietly assuming the old world.

### From the workbench

Why ship only three shuffles? Two reasons, one principle. The principle: a
tiny trusted core, with everything else written in the language itself, is
the same discipline Forth has kept for fifty years — the vocabulary grows
from the problem outward, not from the standard inward. The practical
reason: `dup` and `drop` are where the language's linearity rules live
(copying and destroying values are *privileges*, not givens — chapter 7
introduces types for which they are crimes), so the fewer built-in ways to
copy and destroy, the smaller the trusted surface that enforces the rules.

### Exercises

1. **Trace by hand.** For `1 2 3 swap drop dup +`, write the stack state
   after each term, then confirm with `langc --emit=tc`. What is the final
   value, and what would `E3202` have caught if the final `dup` were
   deleted?
2. **Build `nip`.** Define `nip ( i64 i64 -- i64 )` — keep only the top.
   Then define `tuck ( i64 i64 -- i64 i64 i64 )` — a copy of the top
   *under* the original pair. Test both with `check`.
3. **Predict, then run.** In Lab 2.3, change the condition quotation to
   `[ dup 0 < ]` and the limit to `10`. Predict what `count-up` returns and
   whether the marker still comes out. The loop never runs — explain why
   using the words *condition* and *initial state*.

---

*Next: chapter 3 — Types. Making illegal values unrepresentable: the
primitive types, arrays of fixed size, and range subtypes that carry their
own runtime checks — the `SUBTYPE_FAIL` trap, met properly this time.*
