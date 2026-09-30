# Chapter 9 — Factoring: Style Becomes Proof

> Until you can name it, you cannot check it.
> Until you can check it, you were only hoping.
>
> — workshop wall

## 9.1 The Practice

> **The Practice.** *Factor so each word can be described in one sentence —
> and so every proof the checker makes attaches to the smallest unit that
> can carry it. How a program is divided decides where its claims live, and
> claims that live at interfaces get checked at every call.*

This is the craft chapter. Chapters 3 through 8 supplied the checking
machinery; this chapter is about the human half — the act of dividing a
program into words — and the claim that makes factoring something more
than tidiness in Tyu: **the division changes what can be proven, and
where.**

The homage is explicit. Thinking Forth's factoring chapter taught that
good programs are re-divided constantly — that factoring is not a phase
but the material of programming. Forty years later, in a language with a
checker, the same craft gains a new property: a well-factored word has an
interface, and interfaces are the things this book's machinery has been
checking all along.

## 9.2 The criteria

The one-sentence test comes first, unchanged from the Forth tradition: if
a word's behavior cannot be described in one sentence, it is more than one
word. `bounded-by` — met below — is "does this value stay under this
ceiling, keeping the value?" One sentence, one word, one shape the checker
understands.

Thinking Forth's four diagnostic questions refine the test, and they
translate directly. A word that needs a compound sentence to describe
(*"read the sensor and then if it is stale also reset it"*) is two words.
A word whose description needs time words (*"first this, then that"*) is a
sequence pretending to be a unit. A word whose object is vague
(*"process the data"*) has no single subject, which in Tyu means no
truthful stack effect. A word described by *"initialize…"* is a coincidental bundle
— things that happen to run at the same time, which is chapter 5's
`resource`-without-a-rule smell, promoted to a design review question.

And the counterweight, also from the tradition: **do not factor for
factoring's sake.** A one-use shim around `1 +` adds a name without adding
a sentence. The criteria are clarity, reuse, and — Tyu's addition —
provability. A word that fails all three should stay inline.

## 9.3 Factoring has a grammar

Here is where the craft becomes exact. In chapter 4, the two-sided range
contract was written inline, with the `if` combinator carrying the shape:

```tyu
needs [ dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ]
```

The natural refactor — "factor the test into a word" — writes itself:

```tyu
: in-range ( i64 -- bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;
```

And the checker rejects it: E3220. The word consumes its input and returns
one boolean — net 0 — but a contract predicate must end with the value
*still present* under the verdict (net +1, chapter 4's first shape rule).
The factoring that reads perfectly is unshapable.

The working factoring changes the helper's signature to say what the
predicate's rules actually demand — *keep the value, add the verdict*:

```tyu
: bounded-by ( i64 i64 -- i64 bool )
  => hi dup hi <= ;

: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ 100 bounded-by ] [ false ] if ]
  as i64 ;
```

`bounded-by` is "is the top value at most this ceiling — keeping the
value?" One sentence. Its shape — value in, value and verdict out — is
exactly the shape a predicate can call, so the contract now reads as a
sentence too: *a copy of me is at least zero; if so, am I bounded by 100?*
Lab 9.1 runs it; the boundaries (`0`, `100`) pass and out-of-range still
traps, identically to chapter 4's inline version.

The lesson generalizes beyond contracts: **factoring has a grammar, and
the grammar is the shape rules.** A helper that consumes its subject
cannot serve in a predicate. A loop body must be net-zero, so the factored
body of a loop must be a net-zero word (§9.4). An ISR's pieces must fit
under the handler's ceiling (chapter 6). When a factoring fails to
compile, E3220 or its cousins are not obstructing the craft — they *are*
the craft, stated mechanically.

## 9.4 Reused bodies, reused proofs

The second payoff compounds. Lab 9.2 factors a loop body:

```tyu
: tick ( i64 -- i64 ) 1 - ;

: two-loops ( i64 i64 -- i64 i64 )
  => b => a
  a [ dup 0 > ] [ tick ] while
  b [ dup 10 > ] [ tick ] while ;
```

Two loops, different bounds, one body. Each `while` demands a net-zero
body; `tick` is net-zero *by declaration* — the shape was proven once,
where the word was defined, and both loops inherit the proof. Inline the
body twice and the checker proves it twice; factor it and the proof has a
name. This is the chapter's thesis in its smallest form: **factoring is
how proofs get reused.**

It scales. Chapter 6's `countdown` is a net-zero recursion whose shape
survives because each level is stack-neutral; the neutrality is a property
of the words it is built from. Chapter 5's regions are safest when the
allocate-use-destroy arc lives in one word with `performs {alloc}` on
the label, rather than being smeared across a page — the effect
declares itself where the reader can bind it to the code. And chapter 11
will lean on the same principle from the other side: a resource touched
inline in its lock is checkable *because* the grant is lexical, so
factoring respects the context's grain (locked sections stay inline —
that lesson is chapter 5's and it becomes a factoring rule here: **do not
factor across a context boundary.** The bracket is part of the word.)

## 9.5 When the division is the design

Step back and the criteria stop being style advice. A program whose words
pass the one-sentence test is a program where:

- every claim — a stack effect, a contract, an effect label, a bound —
  attaches to a unit small enough to be *true*;
- every reuse of a factored word re-runs its proofs at the new call site,
  for free;
- review happens at the interfaces, and the interfaces are exactly what
  the checker reads.

The god-word — the hundred-line unit that "just does the update" — is the
opposite organization, and chapter 8's post-mortem warned about its
cousin. Its signature hides its effects; its internal claims are
unreachable by callers; its failure modes surface at 3 a.m. in the field.
The factored program does not prevent bugs by magic; it *positions* them —
each one lands on an interface where the machinery of chapters 3 through 8
is waiting.

---

## Labs — Chapter 9

### Lab 9.1 — The factored contract *(green)*

```tyu
# labs/ch09/green-01-factored-contract.mod — Lab 9.1 (green)
# The two-sided range contract, factored: bounded-by keeps the value and
# returns value+verdict — the shape the contract rules demand from a
# helper. The needs clause reads as a sentence and still traps.
# Expected: silent clean run (exit 0, marker emitted, no F).
module FactoredRange;
import platform/linux { platform.io.log };

: bounded-by ( i64 i64 -- i64 bool )
  => hi dup hi <= ;

: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ 100 bounded-by ] [ false ] if ]
  as i64 ;

: main ( -- i64 )
  0 set-range drop
  50 set-range drop
  100 set-range drop
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean — then change `50` to `150` and confirm the factored contract
traps exactly like chapter 4's inline one. Same proof, new shape, one
sentence.

### Lab 9.2 — One body, two proofs' worth of loop *(green)*

```tyu
# labs/ch09/green-02-reused-loop-bodies.mod — Lab 9.2 (green)
# One net-zero body, two provable loops: tick is factored once and both
# whiles inherit its shape for free.
# Expected: silent clean run (exit 0, marker emitted, no F).
module ReusedBodies;
import platform/linux { platform.io.log };

: tick ( i64 -- i64 ) 1 - ;

: two-loops ( i64 i64 -- i64 i64 )
  => b => a
  a [ dup 0 > ] [ tick ] while
  b [ dup 10 > ] [ tick ] while ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  3 12 two-loops
  => y => x
  x 0 == check
  y 10 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean. Then add a third loop over another bound — it inherits
`tick`'s net-zero proof the moment it compiles. That inheritance is the
lab.

### Lab 9.3 — The factoring the grammar refuses *(red)*

```tyu
# labs/ch09/red-01-shape-demands-value.mod — Lab 9.3 (red)
# The natural-looking factoring: a word that answers "in range?" — but it
# consumes its subject, and a contract predicate must keep the value and
# return value+verdict. Expected: rejected at compile time — E3220.
module NaturalButWrong;

: in-range ( i64 -- bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: judge ( i64 -- i64 )
  needs [ in-range ]
  as i64 ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `in-range` is a fine *word* — one sentence, reusable, readable —
but it is the wrong *shape* for a predicate: the subject is consumed, and
a contract must keep the value under its verdict. The working factoring is
Lab 9.1's `bounded-by`, whose signature `( i64 i64 -- i64 bool )` states
the grammar: value stays, verdict arrives. Fixing the red lab by hand —
before looking at 9.1 — is the exercise.

### Post-mortem — the god-word and the shim *(optional — for readers with C or embedded scars)*

Two anti-patterns, one principle. The god-word: the four-hundred-line
`do_update()` whose signature is `int do_update(ctx*)` — one parameter
hiding twelve effects, no claim small enough to be true, every bug a field
finding. The shim: `int get_two()` that returns `2` — a name with no
sentence behind it, one call site, and a reader who must open it to learn
nothing. God-words hide the claims; shims bury them.

Tyu's checker referees both. The god-word's effects, bounds, and contracts
must all be declared on one unit — and the declarations are either true
(`⊤` somewhere, effects everywhere, budgets refused) or they are lies the
checker catches. The shim compiles but earns nothing: no reuse, no new
proof, no sentence — and the criteria of §9.2 are the review checklist.
Between them is the factored word: one sentence, one interface, proofs
that attach where they can be reused.

### From the workbench

A fair challenge: if the shape rules are associative (chapter 6's monoid),
does factoring change *anything* the checker proves? The shapes of the
parts compose to the shape of the whole either way — that is the monoid's
promise, and it means factoring can never *hide* a violation. What
factoring changes is where claims live and which callers see them. Inline,
a net-zero body is proven in one context; factored, it is proven at a
named interface and re-checked at every use. Contracts factor only along
the grammar the shape rules allow. Effects label the smallest words that
perform them. Nothing about that is cosmetic: the difference between a
claim that is checked once at a named interface and a claim that must be
re-argued at every use site is the difference between a specification and
a hope.

### Exercises

1. **Factor `dist2`.** Chapter 2's `dist2` computes two squared deltas.
   Factor the delta-square into a word, predict its `(net, high)`, and
   confirm both the word and `dist2` still check. Where did the proof of
   "one plate at peak" move to?
2. **Two-sided, reused.** Write `bounded-by`'s mirror, `at-least ( i64
   i64 -- i64 bool )` — "is the top value at least this floor, keeping
   the value?" — and re-express Lab 9.1's contract with both helpers. The
   clause should read: *at least 0, bounded by 100.*
3. **The one-sentence audit.** Take any three words from your own earlier
   labs and apply the one-sentence test and the four diagnostic questions.
   For each: one sentence describing it, and a verdict — keep, inline, or
   split — with the shape-rule consequences of the verdict named.

---

*Next: chapter 10 — Decomposition: Information Hiding, Enforced. The
`.mod`/`.def` boundary, imports and exports, separate compilation — and
the Parnas rule run by a machine: modules hide what might change.*
