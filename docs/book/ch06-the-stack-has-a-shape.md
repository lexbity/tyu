# Chapter 6 — The Stack Has a Shape

> Every phrase has a height.
> Most languages measure it at 3 a.m., in the field, with a broken device.
> Tyu measures it at compile time, on paper.
>
> — workshop wall

## 6.1 The Practice

> **The Practice.** *Every phrase has a shape — what it adds, what it
> removes, and how high it reaches. The checker computes the reach for
> every word, composes it through every call, and refuses programs whose
> reach cannot be bounded.*

Chapter 2 taught the pencil simulation: read left to right, track the
stack. This chapter promotes that habit from a reading skill to a *proof*,
and then shows the proof's exact edge, as an experiment.

## 6.2 net and high

Two numbers describe any phrase's effect on the stack's size:

- **net** — the change in depth: what it leaves minus what it takes.
- **high** — the *peak* depth reached while it runs, measured relative to
  where it started.

`drop` has net −1, high 0 (the stack ends lower than it began; it never
rose). `dup` has net +1, high 1. The phrase `dup drop` is the interesting
one: net 0 (ends where it started) but **high 1** — it rose by one plate in
the middle. Two phrases with identical signatures can have different
shapes, and the peak is the number that decides whether a program fits in
the memory a device actually has.

The power comes from the composition law. For two phrases run in order:

```text
net  (A ; B) = net A + net B
high (A ; B) = max( high A ,  net A + high B )
```

The second line deserves a slow read: B's peak is *shifted up* by whatever
A left behind. Running `dup dup` after `dup` reaches higher than `dup dup`
alone — not because either phrase changed, but because each starts from
where the last ended. The law composes arbitrarily: three phrases, thirty,
a whole word body, a whole call tree — the same two-line rule folds them
all. It is associative (the compiler's own test suite checks this the way
chapter 4's shape rules are checked), and it has an identity — the empty
phrase, `(net 0, high 0)` — which makes the shapes a *monoid*: a structure
that can be folded over any program, bottom-up, in one pass.

Branches merge by taking the worst: an `if` whose arms reach 3 and 9 has
high 9 — the bound must hold on every path, so the bound *is* the worst
path. Loops must be net-zero (chapter 2's rule, now with its reason: a loop
whose body grew the stack would have an uncomputable peak), so a loop's
shape is its body's shape, once.

And there is a third value in the system, for the phrase whose reach has no
bound: **top**, written `⊤`. Unbounded is not a big number; it is a
different answer, and it is absorbing — compose anything with ⊤ and the
result is ⊤. One unbounded word anywhere below makes everything above it
unbounded — that is the answer, not a failure to answer.

## 6.3 The shapes, in person

Lab 6.1 runs four phrases, each a named shape:

```tyu
: peak-10 ( -- )
  1 2 3 4 5 6 7 8 9 10
  drop drop drop drop drop
  drop drop drop drop drop ;
```

`peak-10`: net 0, high 10. Ten pushes, ten drops — the signature says
"nothing happens," and the shape says "…on a stack ten plates tall." The
gap between a signature's net and a body's high is *exactly* the gap this
chapter exists to measure.

```tyu
: symmetric-branch ( -- )
  true
  [ 1 2 3 4 5 drop drop drop drop drop ]
  [ 6 7 8 9 10 drop drop drop drop drop ]
  if ;
```

`symmetric-branch`: both arms peak at 5, so the branch peaks at 5. Make one
arm push six and drop five — a one-plate asymmetry — and the word's high
grows by one even though its net never changes. Worst path, by rule.

```tyu
: loop-temp ( -- )
  1 [ dup 0 > ] [ 1 - ] while drop ;
```

`loop-temp`: the body copies the counter, asks, decrements — net zero per
lap, one temporary plate at the peak. A thousand laps, same height. This is
what "net-zero body" buys: loops get *iteration* without *growth*.

```tyu
: countdown ( i64 -- i64 )
  dup 0 <= [ ] [ 1 - countdown ] if ;
```

`countdown` — recursion, and the chapter's crux. Read its shape per level:
`dup` copies, the comparison consumes the copy, and each branch is net 0 —
the true branch does nothing, the false branch subtracts one and recurses
with the same net. Level by level, the data stack rises by *nothing*. The
monoid folds the self-reference without flinching: the shape stays finite, because
each level is stack-neutral by construction.

## 6.4 The shape rules refuse growth

What happens if the recursion *does* grow — if each level keeps a copy
alive below the call? Try to write it (Lab 6.2):

```tyu
: boom ( i64 -- i64 )
  dup 0 >= [ dup 1 - boom ] [ dup ] if ;
```

Each false-branch pass and each true-branch level leaves one extra plate —
both branches have net +1 — against a signature declaring net 0. The
checker rejects the word: E3220, the declared-versus-computed mismatch from
chapter 2, now protecting something much bigger than a forgotten `drop`.
The same refusal meets a `while` body that grows (Lab 6.3, E3257): the
loop-shaped and recursion-shaped routes to *the stack that never stops
rising* are both compile errors.

Sit with that for a moment, because it is the chapter's central claim: on
the data stack, **unbounded growth is not a runtime event. It is a
sentence at compile time.** The classic embedded death — deep recursion,
silent growth, corrupted memory far from the cause — cannot be written in
the first place. The runtime still keeps a trap (`STACK_OVERFLOW`, code 10)
as a second line of defense, but from checked code, the shapes that would
cause it are refused before they exist.

## 6.5 The handler's own bound

Interrupt handlers play by a stricter rule, for the sharpest reason: an ISR
runs on its *own* stack region, stolen from the middle of whatever `main`
was doing, and its depth budget is set at the binding site where the
handler meets its vector. So its shape is checked against a ceiling, not
just against infinity — and the check bites (Lab 6.4):

```tyu
@interrupt(TIMER0) : isr ( -- )
  isr
;
```

A self-recursive handler has high `⊤`, and `⊤` exceeds every ceiling —
rejected, E5040. The same logic rejects an unbounded handler shape at its
binding site (E5100), which is how the compiler says: *this interrupt
cannot be allowed, because its stack use cannot be promised.* Chapter 11
returns to ISRs — the vector table, the sharing rule, and where the budgets
come from. Here, take the principle: **the tighter the context, the
smaller the shapes it will accept.**

## 6.6 The engine: the unproven half

Now the boundary, delivered as an experiment rather than a footnote.
`countdown` has a finite data-stack shape — net 0, high 1 per the monoid —
and the proof is true. Run it at depth one hundred million (Lab 6.5):

```console
$ ./build/ch6-engine/image.elf
Segmentation fault (core dumped)
$ echo $?
139
$ tyu run labs/ch06/engine-native-stack.mod …
tyu: NO_COMPLETION — exited with code -1 but no `S\n` marker
```

No trap code. No diagnostic frame. The process died by the operating
system's hand — because each recursive level, while stack-neutral on the
*data* stack, is a **native call**: it takes a frame on the CPU's own
return stack, and nothing in the current compiler measures that stack. Ten
million frames later, the OS ends the experiment.

This is the boundary of the proof, and the book's contract says to show it
plainly. The compiler's stack analysis proves the **data stack** — the
stack the language owns, with its base, its limit, and its high-water
cell. The **native stack** — exception frames, call frames, the CPU's own
plumbing — is a separate obligation, analysis for it is a committed
workstream, and until it lands, a bounded shape is a proof about one stack
in a machine with two. The distinction between *proven* and *unproven* is
the whole game in safety work: chapter 11 will rely on the data-stack
proof for ISR budgets; nothing in this book will claim the native half.

Read the engine lab once more for its real lesson. The data-stack proof
held — every plate accounted for, exactly as computed. The crash came from
the stack the proof never claimed. A proof that scopes itself precisely is
not weaker than a guarantee that overreaches; it is the only kind that can
be checked.

## 6.7 Interlude — watching the measurement *(QEMU)*

The compiler *claims* a shape; the runtime can *measure* one. On the
QEMU track (`x86_64-unknown-none` — the first of this book's marked
interludes; install QEMU per Appendix C), the bare-metal runtime tracks the
data stack's high-water cell while the program runs and emits it as an
`H` marker when the image finishes. The harness then does what this whole
chapter does by hand: it compares the measured peak against the compiler's
declared bound. Lab 6.6 runs the book's first QEMU fixture:

```console
$ tyu test --manifest=labs/ch06/manifest.toml \
      --target=x86_64-unknown-none --filter=deep_stack
    ran=1 ([deep_stack])
test result: ok. 1 passed; 0 failed
```

One passed means: compiled, ran to the marker, every check green — *and*
the runtime's measured peak fit inside the compiler's claimed bound. The
harness treats a mismatch as what it is — an unsoundness, a hard failure —
not as a warning to be ignored. It also reports which test *axes* a
manifest leaves uncovered, because a test suite that does not say what it
does not test is just a longer kind of guess.

The measurement is the monoid, checked by the machine: declared on paper,
verified in silicon (well — in a simulator; chapter 8's interludes get
closer to metal than that).

---

## Labs — Chapter 6

### Lab 6.1 — Four shapes *(green)*

```tyu
# labs/ch06/green-01-stack-shapes.mod — Lab 6.1 (green)
# Three shapes and one recursion: a tall peak, symmetric branches, a loop's
# temporary, and a net-zero countdown — each with a bounded (net, high).
# Expected: silent clean run (exit 0, marker emitted, no F).
module Shapes;
import platform/linux { platform.io.log };

: peak-10 ( -- )
  1 2 3 4 5 6 7 8 9 10
  drop drop drop drop drop
  drop drop drop drop drop ;

: symmetric-branch ( -- )
  true
  [ 1 2 3 4 5 drop drop drop drop drop ]
  [ 6 7 8 9 10 drop drop drop drop drop ]
  if ;

: loop-temp ( -- )
  1 [ dup 0 > ] [ 1 - ] while drop ;

: countdown ( i64 -- i64 )
  dup 0 <= [ ] [ 1 - countdown ] if ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  peak-10
  symmetric-branch
  loop-temp
  1000 countdown 0 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean. Then, for each of the four words, write its `(net, high)` on
paper using §6.2's law — including *why* `peak-10`'s high is 10 while its
net is 0, and why `countdown`'s high does not grow with its argument.

### Lab 6.2 — The shape that cannot exist *(red)*

```tyu
# labs/ch06/red-01-accumulating-shape.mod — Lab 6.2 (red)
# Each pass keeps a copy alive below the recursive call: the branches have
# net +1 against the declared net 0. The shape rules refuse the growth
# before it exists. Expected: rejected at compile time — E3220.
module Accumulating;

: boom ( i64 -- i64 )
  dup 0 >= [ dup 1 - boom ] [ dup ] if ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** both branches net +1; the signature promises net 0; chapter 2's
declared-versus-computed check fires — and in doing so refuses a recursion
whose *data-stack* growth would be unbounded. The compile error is the
safety property.

### Lab 6.3 — The loop that leaks *(red)*

```tyu
# labs/ch06/red-02-while-grows.mod — Lab 6.3 (red)
# The body pushes one value per lap (dup 1 - has net +1): a loop whose
# stack grows forever, rejected before the first lap.
# Expected: rejected at compile time — E3257 (loop body must be net-zero).
module WhileGrows;

: leaky ( i64 -- )
  [ dup 0 > ] [ dup 1 - ] while drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `while` demands a net-zero body — chapter 2's rule, now with its
reason attached. A body that nets +1 per lap has a peak that depends on the
lap count, and no fixed number can be a bound for every run.

### Lab 6.4 — The handler that cannot promise *(red)*

```tyu
# labs/ch06/red-03-self-recursive-isr.mod — Lab 6.4 (red)
# An ISR must have a finite stack bound: a self-recursive handler is
# unbounded (high = top) and is rejected at the binding site.
# Expected: rejected at compile time — E5040.
module RecursiveIsr;

@interrupt(TIMER0) : isr ( -- )
  isr
;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** an interrupt handler runs on its own stack, against a budget set
where the handler binds to its vector. A self-recursive handler has
high `⊤`, and `⊤` does not fit under any ceiling. Note this rejection is
checked on the hosted target too — the rule lives in the checker, not in
the hardware.

### Lab 6.5 — The unproven half *(engine, boundary)*

```tyu
# labs/ch06/engine-native-stack.mod — Lab 6.5 (engine, boundary)
# countdown's data-stack bound is finite (net 0, high 1) — the proof holds.
# But the compiler does not yet flatten this recursion to a loop: every
# level is a native call, and at depth 100,000,000 the OS kills the
# process. No trap, no diagnostic: this is the unproven half.
# Expected: compiles cleanly; data-stack proof holds; native stack dies
# (Segmentation fault); tyu run reports NO_COMPLETION.
module NativeBoundary;
import platform/linux { platform.io.log };

: countdown ( i64 -- i64 )
  dup 0 <= [ ] [ 1 - countdown ] if ;

: main ( -- i64 )
  100000000 countdown drop
  "S\n" platform.io.log
  0 ;
export { main };
end;
```

```console
$ ./build/ch6-engine/image.elf
Segmentation fault (core dumped)
$ echo $?
139
$ tyu run labs/ch06/engine-native-stack.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch6-engine
tyu: NO_COMPLETION — exited with code -1 but no `S\n` marker
```

Compare with every engine lab so far: chapters 1, 3, 4, and 5 died with
*named* verdicts (20, 21, 20, HANG). This death has no name from the
runtime — because it is not the runtime's stack that died. The data-stack
proof held; the native stack was never in the proof. One sentence per
column: what was proven, what was not, and which one failed.

### Lab 6.6 — Watching the measurement *(interlude — QEMU)*

Fixture: `labs/ch06/interlude-deep-stack.mod` — the repo's deep-stack
fixture (it writes its markers through the bare-metal `testio` words, so
it runs on the `x86_64-unknown-none` QEMU track, not hosted). Give the
test driver a one-fixture manifest:

```toml
# labs/ch06/manifest.toml
[fixtures]
ignore = []

[[fixture]]
name = "deep_stack"
file = "interlude-deep-stack.mod"
axes = ["deep-stack"]
requires = []
```

```console
$ tyu test --manifest=labs/ch06/manifest.toml \
      --target=x86_64-unknown-none --filter=deep_stack
    ran=1 ([deep_stack])
test result: ok. 1 passed; 0 failed
```

Under QEMU, the bare-metal runtime tracked the data stack's high-water
cell for the whole run and emitted it as an `H` marker at completion; the
harness compared the measured peak against the compiler-claimed bound and
found it inside. The harness's own alarm is one-directional on purpose: a
measured peak *over* the declared bound is treated as unsoundness — a hard
failure — because a bound that can be exceeded was never a bound.

### Post-mortem — the recursion that looked like a loop *(optional — for readers with C or embedded scars)*

The C shape of this chapter's post-mortem is a fatigue-hardened reviewer's war story:
a parser recursive over user input, tested to depth twenty, shipped to
depth a million — the native stack gone, memory corrupted *far* from the
cause, and the crash dump pointing at an innocent `memcpy` three frames
away. C measures neither stack, at neither time: data-stack growth and
native-stack growth are both invisible until the device is a brick.

Tyu splits the problem in two, and the split is the lesson. The data-stack
half is *refused at compile time* — Labs 6.2 and 6.3 are the shapes that
would grow, each one a rejected program. The native-stack half is, today,
*unproven* — Lab 6.5 shows the residual risk running to the OS's verdict.
A split this precise is more useful than a blanket guarantee: it says
exactly what is safe, exactly what is not, and exactly which workstream
closes the gap.

### From the workbench

Why a monoid? Because folding is checking, and checking must not depend on
how the program is parenthesized. The composition law is associative — the
compiler's test suite verifies the identity and associativity laws by
name, the way chapter 4's shape rules are enforced by code — so a word's
shape comes out the same whether it is analyzed alone or inlined into its
caller. ⊤ is absorbing for the same reason a proof must be: one unbounded
callee makes the caller's bound a guess, and guesses do not ship. The
computed bound is not a report for humans; it is stamped into the module's
interface, hashed into its ABI identity, carried to the loader, and — on
the QEMU track — measured against the runtime's own high-water cell. A
claim this central should not be able to fall out of sync with anything,
and this one cannot.

### Exercises

1. **Fold by hand.** For the phrase `1 dup dup drop drop drop`, compute
   `(net, high)` step by step using the composition law — including the
   intermediate peaks. Then confirm the final shape with a five-line
   program and `langc --emit=tc`.
2. **The asymmetry experiment.** In Lab 6.1's `symmetric-branch`, change
   the true branch to push six values but keep its five drops. Predict the
   new high (the false branch did not change!), run, and explain why the
   word got taller without getting deeper.
3. **Two stacks, one sentence each.** Write the two-sentence summary of
   Lab 6.5 for a project status report: what the compiler proved about the
   program's stacks, what it did not, and which sentence a safety
   reviewer will insist stays in the report.

---

*Next: chapter 7 — Places, Borrowing, Ownership. Who may write, when, and
for how long: `&` and `&!`, the scoped borrow blocks, and the `iso` types
that move instead of being copied — the linearity rules that make `dup`
and `drop` privileges rather than rights.*
