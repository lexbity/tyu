# Chapter 7 — Places, Borrowing, Ownership

> Copying is a decision someone made.
> Ownership is a decision someone has to answer for.
>
> — workshop wall

## 7.1 The Practice

> **The Practice.** *A place has one address and a rule: one writer or many
> readers, never both. A pointer borrows a place and lives exactly as long
> as its block. Custody moves — it is never cloned. The checker keeps the
> ledger, and the ledger has no rounding errors.*

Chapter 2 closed with a promise: `dup` and `drop` are privileges, not
rights, and chapter 7 is where the bill arrives. This is the chapter where
Tyu's pointer story — deliberately postponed for five chapters — turns out
to be small, sharp, and already half-familiar.

One note before the doors open: there is no engine lab in this chapter.
Every failure it teaches is a *compile* failure. That absence is the thesis.

## 7.2 Places and pointers

A **place** is a location the language can name: a local bound with `=>`,
a field of a struct, a resource, an element of an array. A **pointer** is a
value that names a place from afar — and in Tyu, a pointer is an ordinary
value on the data stack: it can be pushed, consumed by words, handed to
helpers, stored. Two borrowed flavors exist:

- `& place` — a **shared** borrow, type `^T`: read access.
- `&! place` — a **mutable** borrow, type `^!T`: write access.

Pointers are read and written through *typed* load and store words — `@i64`
loads one signed 64-bit value through a pointer, `!i64` stores one — so a
pointer always remembers what kind of value lives at its place. The type
system of chapter 3 does not stop at the pointer; it goes through it.

Lab 7.1 is the runnable story, and it is five lines of real memory
management:

```tyu
64 as usize platform.mem.region-create
dup 16 as usize platform.mem.region-alloc
dup 42 as i64 !i64
dup @i64 42 == check
drop
platform.mem.region-destroy
```

Create a region, allocate sixteen bytes — `region-alloc` returns a mutable
pointer — store `42` through it, load it back through the same pointer,
compare, drop the pointer, destroy the region. No garbage collector, no
hidden heap, no allocator surprises at 3 a.m.: one region, one lifetime,
one `destroy`, all visible as text. This is chapter 5's `alloc` effect
paying its rent.

## 7.3 The two borrows

The rule that runs the whole chapter fits in one line: **one writer or many
readers, never both.** Any number of shared borrows (`&`) may be alive at
once. A mutable borrow (`&!`) demands to be alone.

The checker enforces the rule with a ledger. Mint a `&!` and the ledger
records the place as *held for writing*; any conflicting mint while the
first is alive is a compile error. Hold a `&!` and `dup` it, and the ledger
rejects the copy — duplicating a writer would de-anonymize the rule
(E5022, verified: `Counter lock [ &!Counter dup … ]` does not compile).

What the ledger *permits* matters as much. Sequential uses are free — close
one borrow, open the next, as many times as the program likes (Lab 7.3).
Shared-plus-shared coexists. The rule is not "touch things rarely"; it is
"never let a writer and a reader disagree about the truth at the same
instant." E5021, Lab 7.8, is the shape of the refusal: two live mutable
mints of the same root, one block, no chance of compiling.

## 7.4 The block is the scope

Borrows from arrays, slices, and spill-temporaries are minted by the scoped
borrow blocks:

```tyu
: reborrow ( i64'4 -- i64'4 )
  &[ drop ] &[ drop ] ;
```

Inside `&[ … ]`, the borrowed thing is in scope; at the closing bracket it
is gone — consumed, checked, closed. Two properties make the block more
than syntax sugar.

**The borrow cannot escape.** Returning it, storing it into a longer-lived
place, or slipping it out through an early `return` — the checker rejects
every exit that takes the borrow along (E5020, Lab 7.7). A borrow is a
*lease*, and the lease's end is the bracket.

**The block must consume what it borrows.** An empty block over a borrowed
array is rejected by the same code: the borrow has to be *used* or
explicitly closed, not abandoned. Chapter 5's word — grants are lexical —
applies again with force: a borrow's lifetime is visible as text, from the
moment of its mint to the bracket that ends it. There is no borrow whose
extent must be guessed.

## 7.5 iso: values that move

Now the sharpest tool in the drawer. Declare a type `iso` and its values
obey a different law:

```tyu
iso Msg;
```

An `iso` value is **custody**: exactly one owner, at all times, and the
owner is known to the checker. Three privileges vanish, each with its own
code:

- `dup` on an `iso` — E5010 (Lab 7.4). Two copies would be two owners of
  one thing; custody cannot be cloned.
- `drop` on an `iso` — E5011 (Lab 7.5). Custody cannot be silently
  discarded either; an explicit destructor capability is required.
- Use after move — E5012 (Lab 7.6). Bind an `iso` with `=>`, push it once,
  and the name is spent; pushing it again is a phantom.

Why would a language *want* values it forbids copying? Because some values
are not facts — they are **handshakes**. A message in flight, a capability
token, a page of memory being donated to another task: each means something
only while exactly one party holds it. Copy the handshake and both sides
believe; drop it and nobody does. `iso` makes the handshake uncopyable, and
the errors above become the *protocol's* errors, enforced at compile time.

The runnable form of custody is the channel (Lab 7.2):

```tyu
platform.channel.make dup
42 platform.channel.send
platform.channel.recv 42 == check
```

`send` *consumes* the value — a move, not a copy — and `recv` receives it
from the same channel. Between the two words, `42` exists in exactly one
place the checker can name: the channel. Note what `dup` was for: the
*channel handle* may be copied freely — it is a doorway, not the custody.
The value that passed through moved.

## 7.6 The boundary, named

Two scoping notes, so the chapter's claims stay inside their proof.

Field borrows of struct parameters — `&p.x` — typecheck precisely (the
chapter 3 trace), but the x86 backend cannot yet emit them (E8008), so this
chapter's runnable labs borrow through *regions and resources*, which emit
and run. The borrow model does not care which; the boundary is in codegen,
on the roadmap. And the manuals' rule that a live borrow forbids
suspension — the borrow's lease and the scheduler's yield cannot overlap —
is part of the same matrix as chapter 5; exercising it end-to-end waits on
that same codegen boundary.

---

## Labs — Chapter 7

### Lab 7.1 — A pointer's whole life *(green)*

```tyu
# labs/ch07/green-01-pointer-roundtrip.mod — Lab 7.1 (green)
# A pointer is an ordinary value on the data stack: region-alloc returns
# one, !i64 stores through it, @i64 loads back, and the region is destroyed.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Pointers;
import platform/linux { platform.io.log };
import platform/mem;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  64 as usize platform.mem.region-create
  dup 16 as usize platform.mem.region-alloc
  dup 42 as i64 !i64
  dup @i64 42 == check
  drop
  platform.mem.region-destroy
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean. Then count the pointer's appearances: minted by `alloc`,
consumed by `!i64`, re-read by `@i64` — and find the `dup`s that make the
arithmetic work. The pointer is data; the ledger tracks it like data.

### Lab 7.2 — Custody through a channel *(green)*

```tyu
# labs/ch07/green-02-channel-custody.mod — Lab 7.2 (green)
# send CONSUMES the value (a move, not a copy); recv receives it on the
# other end of the same channel. Custody transfers, end to end.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Custody;
import platform/linux { platform.io.log };
import platform/channel;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  platform.channel.make dup
  42 platform.channel.send
  platform.channel.recv 42 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean. Then answer before touching the code: why does `dup` before
`send` not violate custody? (What, exactly, got copied — and what did
not?)

### Lab 7.3 — One at a time *(green)*

```tyu
# labs/ch07/green-03-sequential-borrows.mod — Lab 7.3 (green)
# One borrow at a time is always fine: two sequential borrow blocks over
# the same array, each closed before the next opens.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Sequential;
import platform/linux { platform.io.log };

: reborrow ( i64'4 -- i64'4 )
  &[ drop ] &[ drop ] ;

: main ( -- i64 )
  "S\n" platform.io.log 0 ;
export { main };
end;
```

The ledger has no quota — only a rule about *simultaneity*. Ten borrow
blocks in a row over the same array are as legal as one.

### Lab 7.4 — Custody cannot be cloned *(red)*

```tyu
# labs/ch07/red-01-iso-dup.mod — Lab 7.4 (red)
# An iso value is custody; duplicating it would be two owners of one thing.
# Expected: rejected at compile time — E5010.
module IsoDup;

iso Msg;

: bad-dup ( Msg -- Msg Msg ) dup ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `dup` is chapter 2's privilege, and `iso` is the type that does
not hold it. Two copies of a handshake mean two parties who both believe
they were addressed — the protocol error, caught before the protocol runs.

### Lab 7.5 — Custody cannot be dropped *(red)*

```tyu
# labs/ch07/red-02-iso-drop.mod — Lab 7.5 (red)
# An iso value cannot be silently discarded either: dropping one needs an
# explicit destructor capability. Expected: E5011.
module IsoDrop;

iso Msg;

: bad-drop ( Msg -- ) drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** the mirror of Lab 7.4. A value whose existence *matters* cannot
vanish as a side effect of stack discipline; its end must be as deliberate
as its beginning.

### Lab 7.6 — The spent name *(red)*

```tyu
# labs/ch07/red-03-move-twice.mod — Lab 7.6 (red)
# x was moved once (pushed as x); pushing it again is use after move.
# Expected: rejected at compile time — E5012.
module MoveTwice;

iso Msg;

: move-twice ( Msg -- Msg )
  => x
  x x ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** the first `x` moves the value into the word's result; the second
names a handshake that has already been handed over. The ledger marks the
name *spent* at the move.

### Lab 7.7 — The lease ends at the bracket *(red)*

```tyu
# labs/ch07/red-04-borrow-escape.mod — Lab 7.7 (red)
# The borrow lives exactly inside its block; carrying it out (return) is
# an escape. Expected: rejected at compile time — E5020.
module BorrowEscape;

: escape-return ( i64'1 -- Slice(i64) )
  &[
    swap drop
    return
  ] ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** a borrow that outlives its block is a pointer to a place whose
lease expired — the dangling-pointer pattern, refused structurally. The
borrowed slice may be *used* inside; it may not be *kept*.

### Lab 7.8 — Two writers, one place *(red)*

```tyu
# labs/ch07/red-05-conflicting-mints.mod — Lab 7.8 (red)
# Two mutable borrows of the same root alive at once: one writer or many
# readers, never both. Expected: rejected at compile time — E5021.
module ConflictingMints;

resource Counter : i64;

: two-mints ( -- )
  Counter lock [ &!Counter &!Counter drop drop ] ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** two live `&!` mints of one root — the ledger records the first as
holding the place for writing, and the second mint conflicts. Rewrite the
body sequentially (`&!Counter drop &!Counter drop`, consuming each borrow)
and it compiles: the rule was never about the count, only about the overlap.

### Post-mortem — the ghost pointer *(optional — for readers with C or embedded scars)*

The C family of ghosts this chapter exorcises: the iterator invalidated by
the very call that was supposed to append safely; the callback's context
pointer into a struct that was freed on the timeout path; the zero-length
allocation that returns a pointer someone *will* decrement. Each ghost has
the same skeleton — a pointer that outlived its place — and each is found
by the same unreliable method: someone reproduces it eventually.

Tyu's answer is not discipline; it is the ledger. A pointer minted from a
place is a lease whose term is a lexical block (E5020 closes the escape
routes), a mutable place is single-writer for the term of the lease
(E5021/E5022), and a value whose meaning depends on sole ownership cannot
be copied, dropped, or used after it moved (E5010–E5012). The ghosts are
not harder to find; they are unrepresentable.

### From the workbench

Chapter 2 promised that `dup` and `drop` being builtins was about a small
trusted surface. Here is the payoff: *copyability is a property the
checker tracks per type* — the `copyable` capability in the effect matrix —
and `iso` is simply a type for which the capability never grants. The same
closed-set thinking runs the ledger: a fixed-size table, no allocation, no
garbage collector, and no path where "the checker wasn't sure." When the
manual says the borrow rules are enforced by a fixed-capacity ledger, the
embedded reader should hear: the safety machinery itself fits this
language's own rules for embedded code.

### Exercises

1. **Make it legal.** Rewrite Lab 7.8's body so both borrows happen —
   sequentially — and the word compiles. Then explain, using the words
   *ledger*, *live*, and *overlap*, why the fixed version is just as
   useful as the illegal one.
2. **The spent channel.** Declare `iso Parcel;` and write `: deliver
   ( Parcel -- ) drop ;`. Compile it and name the exact code — then
   explain what a correct `deliver` would need (chapter 4's capability
   vocabulary has the word for it: a destructor grant).
3. **Handle versus custody.** Lab 7.2 `dup`s the channel handle but
   Lab 7.4 refuses to `dup` a `Msg`. Write the two-sentence rule that
   separates the cases, using the words *doorway*, *custody*, and
   *copyable*.

---

*Next: chapter 8 — Touching Hardware. MMIO as a type discipline:
register-maps with access modes the compiler enforces, volatile semantics
the backend cannot elide, and the first contact with devices that are
named, not addressed.*
