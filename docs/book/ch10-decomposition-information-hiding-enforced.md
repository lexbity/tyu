# Chapter 10 — Decomposition: Information Hiding, Enforced

> Hide what might change.
> Then let the machine check that you hid it.
>
> — workshop wall, after Parnas

## 10.1 The Practice

> **The Practice.** *A module is a decision about what might change. Its
> interface file is the promise the rest of the program is allowed to know,
> and the checker holds both sides to it: the implementation must match the
> interface, and callers are checked against the interface — whether or not
> the implementation exists.*

Half a century ago, David Parnas proposed the rule that still divides
good systems from bad: decompose according to *what is likely to change*,
and hide each changeable decision behind an interface. Chapters 5 and 7
borrowed his idea for scopes and borrows. This chapter runs it at the
scale of modules — and the Tyu difference is the second line of the
epigraph: information hiding here is not a convention enforced by review
meetings. The `.def` file is checked, the imports are checked, and a
drifted mirror stops the build.

## 10.2 The two files

Every module can be two files:

- **`Math.mod`** — the *implementation*: word bodies, private helpers, the
  machinery.
- **`Math.def`** — the *interface*: which words are exported and exactly
  what they promise. Signatures — and, as §10.5 shows, the safety
  contracts too.

Lab 10.1 is the smallest true split in the book:

```tyu
# Math.mod — the implementation
module Math;

: double ( i64 -- i64 )
  2 * ;

export { double };
end;
```

```tyu
# Math.def — the interface
module Math;
export { double };
: double ( i64 -- i64 ) ;
end;
```

The interface is the whole program as far as a caller is concerned: one
word, one stack effect, no bodies. The caller module imports by name:

```tyu
# App.mod — the caller
module App;
import Math { double };
…
: main ( -- i64 )
  21 double 42 == check
  …
```

Build it by pointing the driver at the caller and the directory that holds
the pieces:

```console
$ tyu build labs/ch10/green-01/App.mod -I labs/ch10/green-01 \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch10
$ ./build/ch10/image.elf
S
```

The driver resolves `Math` through the include path — implementation
first, interface if no implementation exists — builds each module, and
links. One command, two modules, one boundary.

## 10.3 The interface suffices

Here is the property that makes this *enforced* information hiding rather
than polite information hiding. Delete the implementation entirely — keep
only `Math.def` in the directory — and the caller still typechecks
completely:

```console
$ langc --emit=ir labs/ch10/green-02/App.mod -I labs/ch10/green-02
format_ver 6
module App
…
```

Lab 10.2 is exactly that: a caller, an interface, and no implementation
anywhere in the directory — and the checker verifies every call against
the interface's promises. This is Parnas's test, mechanized. If the
caller can be fully checked against the interface alone, then the
interface is *sufficient* — nothing about the implementation leaked into
the caller's assumptions. A team can ship the `.def` to every consumer
and keep changing the `.mod` freely; chapter 13's dynamic module loading
is this property, extended to runtime.

## 10.4 The mirror must not drift

The interface is a promise about the implementation, so the two are
compared — and the comparison has teeth. Three ways to drift, three
rejections:

**The signature drift.** The interface promises `( i64 -- i64 i64 )`; the
implementation produces one value. Building the pair fails (Lab 10.3):

```console
error[E2218]: interface/implementation word signature mismatch
```

This is the "forgotten mirror" — the anti-pattern where someone changes
the implementation and the interface file, edited by hand a month ago,
quietly describes a program that no longer exists. Chapter 1's
post-mortem was this bug in comment form; here it is a build failure.

**The import past the wall.** The implementation has a `hidden` word; the
interface does not export it; the caller imports it anyway (Lab 10.4):

```console
error[E2203]: imported symbol not exported by interface
```

`hidden` *exists* — the compiler can see it — and the refusal is the
design working: the interface is the sole channel through which callers
may reach in. If the module's author wanted `hidden` shared, the fix is
to export it deliberately, in the file whose diff everyone reviews.

**The contract drift.** Interfaces carry more than signatures — the
safety clauses ride along. An interface that declares `performs {alloc}`
for a word whose implementation performs nothing is a mismatch (Lab
10.5):

```console
error[E2219]: interface/implementation word effect mismatch
```

The direction of this check matters as much as its existence: an
interface may promise *more* than the caller needs and the caller is
still checked against the promise — which means a module can *widen* its
declared effects to reserve room for a future implementation change
without breaking callers, but never *under*-declare what the
implementation does. Hiding a decision includes hiding the right to make
a different one later; the interface is where that right is reserved.

## 10.5 What travels on the interface

Chapter 4 asked for contracts to be part of the word's interface, and the
`.def` is where that lands: `needs`, `performs`, and the stack bound all
project into the interface record, all of it folded into the word's ABI
identity — the hash that chapter 13's loader will check before a module
is allowed to load. The practical reading for a module author: **your
`.def` is your module's constitution.** Callers compile against it; the
loader authenticates by it; and its diff is the truthful changelog of what the module
promises.

This is also the Modula/Oberon inheritance doing real work. The
definition/implementation split was invented so that a system's
*interface surface* could be smaller than its *code surface*. Tyu's
addition is that the surface is machine-checked in both directions — the
implementation cannot drift from the interface (E2218, E2219), and the
callers cannot reach past it (E2203).

---

## Labs — Chapter 10

### Lab 10.1 — Two files, one boundary *(green)*

Three files: `labs/ch10/green-01/Math.mod`, `Math.def`, `App.mod`
(all listed in §10.2). Build the caller and run:

```console
$ tyu build labs/ch10/green-01/App.mod -I labs/ch10/green-01 \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch10-1
$ ./build/ch10-1/image.elf
S
```

Then the experiment the boundary exists for: change `double`'s body to
`2 +` (or anything legal) and rebuild **App alone**. The caller rebuilds
against the unchanged interface without ever knowing the implementation
moved.

### Lab 10.2 — The interface suffices *(green)*

Directory `labs/ch10/green-02/` contains `Math.def` and `App.mod` — and
no `Math.mod` at all:

```tyu
# labs/ch10/green-02/Math.def — Lab 10.2 (green): the interface alone
module Math;
export { double };
: double ( i64 -- i64 ) ;
end;
```

```tyu
# labs/ch10/green-02/App.mod — Lab 10.2 (green): the caller, no
# implementation present anywhere in this directory.
module App;
import Math { double };

: caller ( i64 -- i64 ) double double ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

```console
$ langc --emit=ir labs/ch10/green-02/App.mod -I labs/ch10/green-02 \
        --sysroot=sysroot
format_ver 6
module App
…
```

The caller typechecks in full against the interface alone. Then break the
caller on purpose — call `double` with the wrong arity — and confirm the
rejection names the mismatch: proof that the checking is real, not a
courtesy pass.

### Lab 10.3 — The forgotten mirror *(red)*

Directory `labs/ch10/red-01/`: the implementation of `double` produces
one value; the interface promises two; `App` imports. Build the pair:

```console
$ tyu build labs/ch10/red-01/App.mod -I labs/ch10/red-01 …
error[E2218]: interface/implementation word signature mismatch
```

**Why:** the interface is not decorative — it is checked against the
implementation on every build of the pair. Fix it in the interface (the
truth is the implementation's) and the build heals; fix it in the
implementation (matching the interface's fiction) and the build heals
*wrong* — which is why the diff that matters is the `.def`'s.

### Lab 10.4 — Importing past the wall *(red)*

Directory `labs/ch10/red-02/`: `Math.mod` defines a `hidden` word but
exports only `double`; `App` imports both:

```console
$ tyu build labs/ch10/red-02/App.mod -I labs/ch10/red-02 …
error[E2203]: imported symbol not exported by interface
```

**Why:** the interface is the sole channel from outside to inside. The
word exists; the wall is the point. The deliberate fix — exporting
`hidden` in `Math.def` — is a reviewed, diffed decision, which is exactly
what an export should be.

### Lab 10.5 — The contract rides on the interface *(red)*

Two files, `labs/ch10/red-03/E.def` and `E.mod`: the interface declares
`: fx ( -- ) performs {alloc} ;` while the implementation performs
nothing:

```console
$ tyu build labs/ch10/red-03/E.mod -I labs/ch10/red-03 …
error[E2219]: interface/implementation word effect mismatch
```

**Why:** contracts are interface, not hygiene — chapter 4's rule, now
visible in the interface file itself. The mismatch fires in both
directions: an implementation that does more than its interface declares
is caught, and one that does less than the interface reserves is caught
too, because the reservation is a promise to callers.

### Post-mortem — the god-module *(optional — for readers with C or embedded scars)*

Chapter 9's god-word grown to file scale: the C module that exports
forty functions and three hundred lines of globals in a header, where
"interface" means "everything is reachable" and "hiding" means "the
comments say which globals are private." Every refactoring must assume
callers touch everything, because they can; every new feature reaches
into a neighbor's data because the wall was never load-bearing. The
module boundary exists in the file system, not in the language — so it
enforces nothing.

The Tyu shape is the inverse: the export list is finite and checked, the
`.def` is the only reachable surface, and the private machinery of
`Math.mod` is invisible to callers *by compile error rather than by
convention*. The refactoring freedom is the payoff: anything not in the
export list can change — body, helpers, algorithms — with the certainty
that no caller anywhere could have depended on it. That certainty is
Parnas's "hide what might change," delivered as a compiler guarantee.

### From the workbench

Why check the interface against the implementation at all? The caller
never sees the implementation — so why care if they disagree? Because
they disagree in one direction only: an implementation that does less
than its interface promises *will* be relied on by a caller, and the
reliance compiles today and traps in the field. The E2218/E2219 checks
are the machine keeping the module author's propaganda consistent with
the module's behavior. And folding the interface into the word's ABI
identity — the same hash discipline as chapters 5 and 6 — means a changed
promise is a *different module* to every consumer: chapter 13's loader
will refuse a module whose promises no longer match the program that
built against them. The mirror, the wall, and the seal are one mechanism.

### Exercises

1. **Split a chapter-2 lab.** Take Lab 2.4's `Over` program and split it
   into `Over.mod`/`Over.def` plus an `App` caller. Then add a *second*
   exported word without touching `App`, rebuild, and confirm the caller
   never needed to know.
2. **The reservation.** Write a module whose interface declares
   `performs {alloc}` on a word whose current implementation performs
   nothing — and confirm E2219 fires. Then give the implementation a
   real region allocation so the interface's reservation becomes true.
   Write one sentence on why a team might reserve an effect before using
   it.
3. **The wall audit.** Lab 10.2's caller typechecks against the interface
   alone. Delete the `export { double };` line from the interface and
   recompile the caller: which error appears, and which of this
   chapter's three refusals is it? (Careful: it is a different one than
   you expect.)

---

*Next: chapter 11 — Shared State and the Cross-Context Rule: Interrupts.
The resource, the lock, the ISR context — and why a system that can prove
its interrupt sharing can also open its doors to dynamically loaded code.*
