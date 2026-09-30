# Chapter 5 — Effects and Capabilities: The Two-Sided Law

> A signature can say what arrives and what leaves.
> The interesting question is what happens *in between*.
>
> — workshop wall

## 5.1 The Practice

> **The Practice.** *Declare what a word does; the caller decides. A word's
> effects are checked against the context that calls it, and a context's
> capabilities belong to the code lexically inside it — so the reviewer
> reads the label on the tin, and the compiler checks that the contents
> match.*

Chapters 3 and 4 fenced in *values*: what they are, and what must be true
about them. This chapter fences in *actions* — the third thing a signature
can promise. Suspending a task, allocating memory, touching hardware,
running forever, running inside an interrupt: these are not values and not
facts, they are **events a word can cause**, and Tyu's rule is that events
are declared, checked, and — where the platform cannot allow them —
forbidden.

## 5.2 The two sets

Every word carries two closed sets in its signature, written as clauses:

```tyu
: pwm-set ( Percent -- )
  performs {mmio}          # what this word DOES
  requires {write(pwm)}    # what this word NEEDS granted
  …
```

**Effects** — the `performs` set — are events the word may exhibit:

| Effect      | Meaning                                    |
|-------------|--------------------------------------------|
| `suspend`   | may yield to the scheduler                 |
| `interrupt` | runs with interrupt-handler semantics      |
| `diverge`   | may never return                            |
| `mmio`      | performs volatile hardware access           |
| `alloc`     | may allocate from a region                  |

**Capabilities** — the `needs {…}` set — are permissions the word requires
the surrounding world to hand it: `write(R)` for a resource `R`,
`suspendable` inside a scheduler handler, `bounded-stack(N)` for entry
points that must prove their stack, `core(N)` for multi-core spinlocks.

Two set relations decide every call site, and they are the law of this
chapter:

```text
CAPABILITY:  needs(word)      ⊆  grants(context)      # everything needed is granted
EFFECT:      performs(word)   ∩  forbids(context) = ∅  # nothing done is forbidden
```

A **context** is a lexical region that grants capabilities and forbids
effects: a word body, a `lock` block, a `platform.task.run` quotation, a
borrow block, an ISR body. Enclosing contexts fold together — grants add
up, forbids accumulate. And because both sets are small and closed, every
cell of the matrix is testable; the compiler's own error codes are the
matrix, made executable.

## 5.3 The matrix

The context × effect table below is the chapter's centerpiece. Two of its
rows are behind this chapter's labs; the ISR row arrives in chapter 11.

| Context              | Grants                | Forbids              | Verified in |
|----------------------|-----------------------|----------------------|-------------|
| word body (`main`)   | —                     | `suspend` (no handler) | Lab 5.7, E5001 |
| `R lock [ … ]`       | `write(R)`            | `suspend`, `interrupt` | Labs 5.2, 5.8 |
| `platform.task.run […]` | `suspendable`      | — (discharges `suspend`) | Lab 5.3 |
| ISR body             | —                     | `suspend`            | chapter 11  |
| `&[ … ]` borrow      | `borrow-live`         | `suspend` (while live) | chapter 7  |

Read one row as a sentence to see the design: inside `R lock [ … ]`, the
code *may write* `R` (that is the grant — without the lock, Lab 5.6 shows
the same access rejected E5004) and *may not suspend* (that is the forbid —
holding a device while yielding is how deadlocks are born; Lab 5.8 shows
the rejection). One row, one deadlock prevented, statically.

Two verified facts about how the checking flows, worth memorizing because
they shape how code decomposes:

- **Declared effects travel with calls.** A word declared `performs
  {suspend}` that is called from `main` triggers the same rejection as a
  bare `yield` there: the caller's context must allow what the callee
  declares (E5001, Lab 5.7).
- **The declaration obligation is local.** Effects generated *directly* by
  a body's own operations (the `loop` that never returns, the primitive
  that allocates) must be covered by that word's own `performs` — E5005,
  Lab 5.5. Effects arising from *callees* are handled at the call through
  the forbid check, not by re-declaration.

## 5.4 Saying what a word does

The label on the tin is a clause on the signature. Lab 5.1's word
allocates a region, rounds-trips one value through it, and destroys it —
and says so:

```tyu
: grab ( -- )
  performs {alloc}
  64 as usize platform.mem.region-create
  dup 16 as usize platform.mem.region-alloc
  dup 42 as i64 !i64
  dup @i64 42 == check
  drop
  platform.mem.region-destroy ;
```

Read the mechanics, because this is the storage story of chapter 7 in
one-line preview: sizes cross word boundaries as `usize` (`as usize` — the
conversion habit from chapter 3), the region handle is `dup`-ed because it
is needed three times, `!i64` stores and `@i64` loads through the allocated
pointer, and the region is destroyed — allocation in Tyu is explicit,
bounded, and *labeled*.

The declaration itself is checked into the word's interface — it is part
of what a caller's context is measured against. The most dramatic effect,
though, needs no platform at all:

```tyu
: spin ( -- ) performs {diverge} [ ] loop ;
```

`loop` runs its (empty, net-zero) body forever. A word containing it *may
never return*, and the `diverge` declaration says exactly that. Lab 5.5
shows the checker demanding the declaration; the chapter's engine lab shows
what a declared divergence *looks like* when the program runs: the image
starts, spins, and never completes — `tyu run` reports
`HANG — image did not exit within 3s` and exits 124. The declaration told
the truth at compile time; the harness told it again at run time.

`diverge` deserves one more sentence, because it will matter in chapter 6:
*may not return* is a fact about **termination**, not about stack depth. A
poll loop that never returns also never grows its stack. Tyu keeps the two
axes separate — `diverge` for termination, `high` for depth — because they
fail in different ways and are proven by different means.

## 5.5 The grant: resources and locks

A `resource` is storage with a rule: it is writable only inside its lock,
and the lock is what *grants* the capability:

```tyu
resource Counter : i64;

: main ( -- i64 )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
  Counter lock [ &!Counter @i64 1 == check ]
  …
```

Inside the first lock, `&!Counter` borrows the resource for writing, one is
added, and the result is stored back through a second borrow. The second
lock re-reads and verifies. Lab 5.2 runs it clean; Lab 5.6 deletes the lock
and watches the checker reject the bare `&!Counter` with E5004.

The grant is **lexical**, and that word is doing real work: it covers the
code written between `[` and `]`, and nothing else. Two consequences,
both verified, both worth designing around:

- **Locked sections do not decompose into helpers.** A word called from
  inside `Counter lock [ … ]` does not inherit the grant in its own body —
  its body is checked in its own default context, where E5004 fires again.
  The lock's contents stay inline. This is a deliberate trade: the grant's
  lifetime is *visible as text*, which is exactly the property that makes
  the cross-context rule of chapter 11 provable.
- **`requires {…}` today is a recorded promise, not an enforced gate.** The
  clause lands in the word's interface and its ABI identity, but the
  current tree enforces capability obligations only through the lexical
  contexts above. Treat `requires` as the label for the world the word
 *expects* — the enforcement story is roadmap.

## 5.6 The discharge

The most elegant row of the matrix is `platform.task.run`. `suspend` is
forbidden in `main` (Lab 5.7) and forbidden in a lock (Lab 5.8) — and yet
tasks yield all the time. The resolution is not an exception; it is a
context:

```tyu
[ "ticked\n" platform.io.log platform.task.yield platform.task.yield ] platform.task.run
```

`platform.task.run`'s quotation is a context that **grants `suspendable`**
and — the crucial move — *discharges* the `suspend` effect: code inside may
yield without any caller above `run` needing to know or care. This is the
"no colored functions" trick, stated structurally: suspension does not
infect every caller up the stack, because the handler is a visible lexical
boundary where the effect ends. Lab 5.3 runs it — the quotation prints,
yields twice to the scheduler, finishes, and `main` continues to the
marker, none the wiser.

## 5.7 Declared forever

The engine lab deserves its own section because it is the chapter's thesis
in runtime form. The program compiles — every declaration in order. It
runs — the image boots. And then it does exactly what its signature said
it might: it never returns.

```console
$ tyu run labs/ch05/engine-declared-diverge.mod … --timeout=3
tyu: HANG — image did not exit within 3s
$ echo $?
124
```

Chapter 1's engines trapped (exit 20, 21); this one *hangs*, by design, and
the harness classifies the hang as its own verdict. Three failure classes,
three plain signals: the lie that traps, the promise that breaks, and the
termination that was declared. Compare with the vigilance model, where a
forever-loop on a device is discovered by a watchdog and a flashlight.

---

## Labs — Chapter 5

### Lab 5.1 — The label on the tin *(green)*

```tyu
# labs/ch05/green-01-alloc-effect.mod — Lab 5.1 (green)
# grab allocates from a region and says so: performs {alloc} is the label
# on the tin, checked into the word's interface. The region is created,
# used for one typed store/load round-trip, and destroyed.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Grab;
import platform/linux { platform.io.log };
import platform/mem;

: grab ( -- )
  performs {alloc}
  64 as usize platform.mem.region-create
  dup 16 as usize platform.mem.region-alloc
  dup 42 as i64 !i64
  dup @i64 42 == check
  drop
  platform.mem.region-destroy ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  grab
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean. Then read it once more as the shape of explicit allocation:
create, use, destroy — with the declaration above telling every reader
which words inside may allocate.

### Lab 5.2 — The grant *(green)*

```tyu
# labs/ch05/green-02-the-grant.mod — Lab 5.2 (green)
# The resource is writable only inside its lock: the lock is what grants
# write(Counter), and the grant covers the code lexically inside.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Counter;
import platform/linux { platform.io.log };

resource Counter : i64;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
  Counter lock [ &!Counter @i64 1 == check ]
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean, then try the §5.5 decomposition experiment: move the
increment into a helper word and call the helper from inside the lock.
Predict E5004 before compiling, and explain it with the word *lexical*.

### Lab 5.3 — The discharge *(green)*

```tyu
# labs/ch05/green-03-the-discharge.mod — Lab 5.3 (green)
# platform.task.run is the handler that discharges suspend: inside its
# quotation the task may yield; when it finishes, main continues.
# Expected: prints "ticked", then the marker; exits 0.
module Discharge;
import platform/linux { platform.io.log };

: main ( -- i64 )
  [ "ticked\n" platform.io.log platform.task.yield platform.task.yield ] platform.task.run
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it. Two yields inside, one marker after — `main` resumed. Then move
one `platform.task.yield` *out* of the quotation, below `run`, and predict
the compile error before building (it is Lab 5.7's).

### Lab 5.4 — Declared forever *(engine)*

```tyu
# labs/ch05/engine-declared-diverge.mod — Lab 5.4 (engine)
# loop is net-zero and never returns: it performs diverge, the word
# declares it, the program runs forever — declared, then observed as HANG.
# Expected: compiles cleanly; hangs; `tyu run --timeout=3` classifies HANG.
module Diverge;
import platform/linux { platform.io.log };

: spin ( -- ) performs {diverge} [ ] loop ;

: main ( -- i64 )
  spin
  "S\n" platform.io.log
  0 ;
export { main };
end;
```

```console
$ tyu run labs/ch05/engine-declared-diverge.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch5-engine --timeout=3
tyu: HANG — image did not exit within 3s
$ echo $?
124
```

Compiles clean, runs forever, classified as HANG with its own exit code.
This lab intentionally has no trap and no marker: the *declared* non-return
is the experiment. Delete the declaration (`performs {diverge}`) and
confirm the compile-time rejection — that is Lab 5.5.

### Lab 5.5 — The undeclared event *(red)*

```tyu
# labs/ch05/red-01-undeclared-effect.mod — Lab 5.5 (red)
# loop performs diverge; performs {} declares none of it.
# Expected: rejected at compile time — E5005 (undeclared effect).
module Undeclared;

: forever ( -- ) performs {} [ ] loop ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `loop` performs `diverge` — the body's own operations must be
covered by the word's own declaration, and `performs {}` claims the empty
set. Fixing it is one word long: `performs {diverge}`, exactly what
Lab 5.4's `spin` says.

### Lab 5.6 — The missing grant *(red)*

```tyu
# labs/ch05/red-02-resource-outside-lock.mod — Lab 5.6 (red)
# The grant comes from the lock, and no lock encloses this access.
# Expected: rejected at compile time — E5004 (capability missing).
module OutsideLock;

resource Counter : i64;

: sneak ( -- i64 )
  &!Counter @i64 ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `&!Counter` asks for `write(Counter)`, and grants exist only
inside `Counter lock [ … ]` — Lab 5.2's lexical rule, seen from the
outside.

### Lab 5.7 — Yielding with nothing to yield to *(red)*

```tyu
# labs/ch05/red-03-suspend-in-main.mod — Lab 5.7 (red)
# yield performs suspend; main is not a suspendable context and nothing
# here discharges it. Expected: rejected at compile time — E5001.
module SuspendInMain;
import platform/linux { platform.task.yield };

: main ( -- i64 )
  platform.task.yield
  0 ;
export { main };
end;
```

**Why:** `suspend` is forbidden in a context with no scheduler handler
above it. `main` is not `suspendable`, nothing lexical here discharges the
effect, and the compiler refuses to build a program whose future it can
already see: a yield with nothing to yield to.

### Lab 5.8 — The deadlock that did not ship *(red)*

```tyu
# labs/ch05/red-04-suspend-under-lock.mod — Lab 5.8 (red)
# A lock forbids suspend: yielding while holding a device is exactly the
# deadlock the rule exists to prevent. Expected: E5001.
module SuspendUnderLock;
import platform/linux { platform.task.yield platform.task.run };

resource Counter : i64;

: poll-under-lock ( -- )
  Counter lock [ [ platform.task.yield ] platform.task.run ] ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** Lab 5.3's `run` *does* discharge suspend — but the lock around it
forbids it, and forbids accumulate over grants. Yielding while holding a
device is the classic "works on my desk, locks the fleet in the field"
bug; here it is a compile error with a row of the matrix as its citation.

### Post-mortem — the action hero *(optional — for readers with C or embedded scars)*

The C function that reviews well and does everything: `sensor_read()` that
also takes a lock; `log_msg()` that also allocates; `timer_start()` that
also enables an interrupt — each one discovered during the incident, never
during the review, because C signatures carry *types* and the actions ride
invisible. The macro that "just" wraps a call plus one innocent counter
increment is the same animal with different fur.

`performs {…}` is the ingredients label: it cannot make a word truthful, but
it moves the actions onto the signature where reviewers actually look, and
the checker holds the label to the contents at every call site. When the
incident report says "the handler suspended while holding the device," the
Tyu answer is Lab 5.8: that program does not compile, and the citation is a
row of a table.

### From the workbench

Why closed sets? Because a safety claim that can be enumerated is a safety claim that
can be tested. Five effects, a handful of capabilities, a short list of
contexts — every cell in the matrix of §5.3 corresponds to a fixture in the
compiler's corpus, and every new effect would have to earn its bit, its
wire encoding in the module format, and its cells. The same discipline that
keeps `dup` and `drop` a tiny trusted core keeps the action vocabulary
small: the language says *no* to new effects so that *yes* stays
meaningful.

### Exercises

1. **Label the tin.** Add `performs {alloc}` to a word that does *not*
   allocate, and a word that calls Lab 5.1's `grab`. Both compile — explain
   what each declaration promises, and which one the §5.4's "local
   obligation" rule would actually demand.
2. **Two locks.** Write `resource A : i64;` and `resource B : i64;` with a
   `main` that locks `A`, then (separately) locks `B` and increments both.
   Then attempt `A lock [ B lock [ … ] ]` and read the rejection. Nested
   locks are rejected (E5002) — explain why "no nesting" is the natural
   consequence of "forbids accumulate."
3. **The pair, in prose.** A teammate proposes moving Lab 5.3's body into a
   helper word `tick ( -- )` so `main` reads `[ tick ] platform.task.run`.
   Predict whether it compiles, using the words *lexical*, *context*, and
   *discharge*. Then verify — the answer is more interesting than yes or
   no, because `tick`'s own body now *performs* suspend and must say so.

---

*Next: chapter 6 — The Stack Has a Shape. The data stack gets its own
proof: the `(net, high)` discipline, what `⊤` means for recursion, and the
{bounded} entry points that make a stack overflow a compile-time event.*
