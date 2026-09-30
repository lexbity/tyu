# Chapter 3 — Types: Making Illegal Values Unrepresentable

> Every value is a claim about the world.
> A type is the claim written where the machine can read it.
>
> — workshop wall

## 3.1 The Practice

> **The Practice.** *Every value has a type, and the type says which values
> make sense and which words may touch them. The checker rejects programs
> that break the promise before they exist — and when a value breaks it at
> run time anyway, the runtime traps with a name, not a shrug.*

Chapter 2 lived on two types: `i64` and `bool`. This chapter widens the
idea — from "numbers have kinds" to *the type is where correctness lives*.

## 3.2 What a type is

A **type** answers two questions about a value: *what can it be?* — the set
of values it might hold — and *what can be done to it?* — the words that
will accept it. `bool` is the type whose whole universe is `true` and
`false`; the only words that accept it are the logic words and `if`. An
`i64` is any signed 64-bit integer; arithmetic accepts it. A `State` —
invented in §3.3 — will be whatever its declaration says it is, and nothing
else.

Go back to the plates of chapter 2 and add one fact: every plate is
*stamped*. When a word runs, it reads the stamps of the top plates before it
touches anything — `+` will take two `i64` plates and refuses a `bool`.
None of this is a runtime scan; it is the checker, replaying the program on
paper, refusing to let the program exist if any word is ever handed the
wrong stamp.

The primitive stamps, all fixed-size, all with the same meaning on every
target:

| Type    | Bytes | Holds                                        |
|---------|-------|----------------------------------------------|
| `u8` `u16` `u32` `u64` | 1 2 4 8 | unsigned integers          |
| `i8` `i16` `i32` `i64` | 1 2 4 8 | signed integers            |
| `usize` `isize` | pointer-width | counting things that live in memory |
| `bool`  | 1     | `true` or `false`                            |

(Pointers — `^T` and `^!T` — complete this table, and they are chapter 7's
subject.)

Two consequences worth noticing now. First, `==` and `<` are declared
`( i64 i64 -- bool )`: they compare `i64`s and produce a `bool` — a
different *kind* of plate, and the checker will not let the two blur.
Second, arithmetic has no `+` for `u32` and `u8`: the working arithmetic
type is `i64`, and other widths arrive and leave through explicit
conversions (§3.6). Small types are for *storage and meaning*, `i64` is for
*computation*.

## 3.3 Enums: a type that is exactly a set

Many values in real systems are not numbers wearing costumes — they are
choices. A motor is idle, running, or faulted. A day is one of seven. An
`enum` writes the whole set down:

```tyu
enum State : u8
  Idle = 0x00
  Run  = 0x01
end;
```

The `: u8` names the *representation* — the byte this choice occupies when
stored. The variants are the type's entire universe: a `State` can be
`State.Idle` or `State.Run`, and that is the list. Variants are pushed by
name, and words can take a `State` as their input type:

```tyu
: announce ( State -- ) drop ;
```

The power of the idea shows up when someone tries to blur it. `State.Run`
looks like the number `1`, and a program full of C habits wants to pass it
where an `i64` is declared. The checker refuses — Lab 3.5 makes the
rejection happen. **An enum is not its number.** The set of legal values is
the type, the type is checked at every call site, and "some other number
that happens to fit in a byte" is not a `State`.

Two notes about the current tree, because this book teaches Tyu as it is. Converting *out* of an enum (`State.Run as u8`) is stubbed — it
produces `0` regardless of variant — and converting *in* with `as? State`
validates against the base type's range, not the variant list. Until those
land on the roadmap, treat an enum as an opaque named type: its power today
is the distinctness, and the distinctness is real and enforced.

## 3.4 Structs: values that travel together

Coordinates of a point, settings of a device, a sensor reading with its
timestamp — values that belong together travel better *inside* one value.
A `struct` declares the grouping:

```tyu
struct Point
  x : i32
  y : i32
end;
```

A `Point` is now a type like any other: words can take one, and return one,
and the checker tracks it through every call — Lab 3.2 runs exactly that.

Field access is **borrow-then-load**: take a borrowed pointer to the place,
then read through it with a typed load word (`@i32` — load one `i32`). The
typecheck trace of a word that sums the fields shows the shape:

```console
$ langc --emit=tc devdocs/book_v3/labs/ch03/green-02-struct-resource.mod \
        --target=x86_64-unknown-linux-gnu --sysroot=sysroot
word sum-fields ( Point -- i64 )
  => | stack:
  & | stack: ptr
  @i32 | stack: i32
  as | stack: i64
  & | stack: i64 ptr
  @i32 | stack: i64 i32
  as | stack: i64 i64
  + | stack: i64
```

Name the struct (`=> p` empties the stack), then per field: `&` borrows the
place, `@i32` loads its `i32`, `as i64` widens it for arithmetic — because
`+` only speaks `i64`, exactly as §3.2 promised.

The boundary, stated plainly: **structs have no literal construction on the
current tree.** A `Point` value arrives as a word parameter, or it sits in
named storage (chapter 7's `resource` story). And borrowing a *field* of a
parameter typechecks but the x86 backend cannot yet emit it (`E8008`) — the
words above run on paper, in the trace, but a `tyu run` lab containing them
will not build. When construction and storage land, the gap closes; the
declaration — the part this chapter is about — is already real.

## 3.5 Subtypes: ranges as types

Here is the chapter's centerpiece, and the most Ada idea in the language.
An `i64` claims to be any of roughly eighteen quintillion values — but a
battery level is *not*. It is an integer **and** it is between 0 and 100.
That second sentence is a type:

```tyu
subtype Percent = i64 range 0 .. 100;
```

A `Percent` is an `i64` with a *narrower universe*. Every value that exists
at a `Percent`-typed place has passed the range check — which means a whole
species of "can't happen" stops being a comment and becomes a wall.

Words do not trade subtypes across their boundaries; they trade the **base
type**, converting explicitly:

```tyu
: take ( Percent -- ) drop ;

42 as Percent take       # convert at the call site — checked
```

`as Percent` is the *checked* conversion: the value is range-trapped at that
moment, and what crosses into `take` has been proven to be a legal `Percent`.
Omit the conversion and the call site does not compile — `42 take` is a type
mismatch (E3212), because an `i64` is not a `Percent` until it has been
checked into one.

The engine demo of this chapter is one line wide. `150 as Percent` compiles
— the checker cannot prove at compile time what `150` will meet at run time,
so it emits the check and lets the runtime speak:

```console
$ tyu run devdocs/book_v3/labs/ch03/engine-broken-range.mod …
tyu: NO_COMPLETION — exited with code 21 but no `S\n` marker
$ ./build/engine3/image.elf; echo $?
21
```

Trap code 21 is `SUBTYPE_FAIL` — the table from chapter 1, kept. The range
was promised in the type, the value lied, and the program died at the exact
boundary of the lie with a named verdict instead of a negative percent
flowing deeper into the system.

For the cases where out-of-range is *expected data* rather than a lie, the
polite conversion asks instead of trapping:

```tyu
150 as? Percent swap drop    # leaves ok=false — a question, not a trap
7   as? Percent swap drop    # leaves ok=true
```

`as?` leaves the converted value *and* an `ok` boolean; the swap-and-drop
keeps just the answer. Soft where the data warrants softness, hard where a
lie is a lie.

One boundary on the idea itself: a word may *take* a `Percent`, but on the
current tree it may not *promise* one — a `-- Percent` output is rejected
(E3220, Lab 3.8). Subtypes live at rest and at checked conversions; words
exchange the base type. The practical style is unaffected: convert in,
operate, convert out.

## 3.6 The three conversions

| Form | Leaves | When it refuses |
|------|--------|-----------------|
| `as T` | the converted value | at run time: traps (21) if the value breaks `T` |
| `as? T` | value **and** `ok` flag | never traps — hands back the answer |
| `bitcast T` | reinterpreted bits | at compile time: E3304 unless sizes match |

`bitcast` is the bare machinery underneath: same bits, new stamp, no
opinion. `u32 bitcast i32` compiles; an 8-byte `i64` `bitcast` into a 4-byte
`u32` does not — the promise cannot be kept, and the checker declines to
invent one (Lab 3.6).

The fourth conversion is the one the language keeps behind a locked door:
raw pointer casts (`as ptr`). They are compile-time errors, E3305 — and on
the current tree they stay errors even under `langc --allow-raw-casts`,
which is the documented escape hatch. The door is marked; today it is also
sealed. Everything the rest of this book needs is reachable through typed
borrows, and chapter 7 is where those live.

## 3.7 What the checker does not yet check

The chapter would be incomplete without its own boundary, since the whole
book's contract is Tyu as it is:

- **`const` declarations parse but are not yet readable** in expressions —
  the name-resolution wire has not been strung (E3210 if tried).
- **Enum conversions** — out is stubbed, in validates the base range only
  (§3.3).
- **Struct construction** does not exist; field borrows of parameters
  typecheck but do not yet emit (E8008, §3.4).

Each line here is a place where the *discipline* outruns the *machinery*.
Write the types as if the machinery were finished — ranges in subtypes,
choices in enums, groupings in structs — because the machinery is the part
that arrives on a roadmap, and the habits are the part that arrives when the
reader decides.

## 3.8 Compile-time discharge

The runtime trap on the range cast is not the whole story anymore: the
compiler also *proves* range facts when it can, and the proof has teeth.
`150 as Percent` is the map of the boundary — the same program behaves
differently under the two build policies:

```console
$ tyu build --verify-policy=open-ok open-cast.mod     # default — builds
verify: obligations — ... provably-failing: cast.0
$ tyu build --verify-policy=no-open open-cast.mod     # the norm, when adopted
tyu: error[E6410]: open obligations (--verify-policy=no-open):
  subtype-range — OpenCast::main::cast.0 line 11
```

Why does the *constant* 150 stay open instead of discharging? Because the
interval engine proves it is **always** out of range — and the trap is the
cast's semantics (§3.4). A discharge would remove a trap that must fire;
the report names it `provably_failing` instead, and the runtime check is
retained. The true pairing is:

- value provably **in** range — discharged, no trap needed (the report
  says `discharged`);
- value provably **out** of range — retained, and the report says
  `provably_failing` (a constant that can never pass);
- value unknown at compile time (a caller's ⊤) — retained, `open`, the
  caller or an external tool must close it.

`--verify-policy=no-open` is the discipline as a gate: `tyu build` fails
(E6410) listing every open obligation with its location. The repository's
own corpus adopts the discipline per-module (`ci/verify-corpus/`,
`ci/verify-allowlist.txt`): every open is justified, and a justification
that stops happening fails `ci/guards.sh G20` (self-cleaning).

## Labs — Chapter 3

### Lab 3.1 — A type that is a set *(green)*

```tyu
# labs/ch03/green-01-enums.mod — Lab 3.1 (green)
# An enum is a distinct type: its variants are pushed by name, and a word
# declared to take a State accepts them — while an i64-expecting word will
# not compile against them (that rejection is Lab 3.5).
# Expected: silent clean run (exit 0, marker emitted, no F).
module Enums;
import platform/linux { platform.io.log };

enum State : u8
  Idle = 0x00
  Run  = 0x01
end;

: announce ( State -- ) drop ;

: main ( -- i64 )
  State.Run announce
  State.Idle announce
  "S\n" platform.io.log 0 ;
export { main };
end;
```

```console
$ tyu run devdocs/book_v3/labs/ch03/green-01-enums.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch3-1
tyu: resolved profile '(implicit all-features-on)' → features: [concurrency, module-loading]
    …
$ echo $?
0
```

### Lab 3.2 — A value that travels together *(green)*

```tyu
# labs/ch03/green-02-struct-resource.mod — Lab 3.2 (green)
# A struct is a distinct composite type. There is no literal construction
# on the current tree, so a Point arrives as a word parameter; words that
# take and return a Point compile and run like any other word.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Structs;
import platform/linux { platform.io.log };

struct Point
  x : i32
  y : i32
end;

: pass ( Point -- Point ) ;

: main ( -- i64 )
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean, then extend it: add a second struct (`Dim`, with `w : i32`
and `h : i32`) and a `resize ( Point Dim -- Point )` word, and confirm the
checker tracks both composites through the signature. The field-loading
reading — borrow, then typed load — is quoted with its real trace in §3.4.

### Lab 3.3 — Ranges as types *(green)*

```tyu
# labs/ch03/green-03-subtypes.mod — Lab 3.3 (green)
# A subtype carries its range in the type. Words exchange the BASE type;
# `as Percent` converts (and range-traps), `as? Percent` asks politely.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Subtypes;
import platform/linux { platform.io.log };

subtype Percent = i64 range 0 .. 100;

: take ( Percent -- ) drop ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  42 as Percent take               # convert at the call site
  42 as Percent as i64 42 == check # round-trip through the subtype
  7 as? Percent swap drop check    # in range: ok = true
  150 as? Percent swap drop not check  # out of range: ok = false
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Four claims, four `check`s, silence from the harness. Then delete the
`as Percent` in the first line (`42 take`) and meet E3212 — the call-site
type mismatch that forces the conversion to be *said out loud*.

### Lab 3.4 — The lie of 150 *(engine)*

```tyu
# labs/ch03/engine-broken-range.mod — Lab 3.4 (engine)
# Expected: compiles cleanly; traps at run time with SUBTYPE_FAIL
# (trap code 21, hosted exit status 21); never emits the marker.
module BrokenRange;
import platform/linux { platform.io.log };

subtype Percent = i64 range 0 .. 100;

: main ( -- i64 )
  150 as Percent drop   # the lie: 150 is not a Percent
  "S\n" platform.io.log
  0 ;
export { main };
end;
```

```console
$ tyu run devdocs/book_v3/labs/ch03/engine-broken-range.mod \
      --target=x86_64-unknown-linux-gnu --sysroot=sysroot \
      --out-dir=build/ch3-engine
tyu: NO_COMPLETION — exited with code 21 but no `S\n` marker
$ ./build/ch3-engine/image.elf; echo $?
21
```

Confirm the three facts: compiles; dies before the marker; exit status 21.
Then swap the lie for `as?` (Lab 3.3's polite form) and note that the
program now *finishes* — same false value, handled by policy instead of verdict.

### Lab 3.5 — An enum is not its number *(red)*

```tyu
# labs/ch03/red-01-enum-distinct.mod — Lab 3.5 (red)
# A State is not its number: passing the variant where an i64 is declared
# is a compile-time type mismatch. Expected: E3212.
module EnumDistinct;

enum State : u8
  Idle = 0x00
  Run  = 0x01
end;

: takes-int ( i64 -- ) drop ;

: main ( -- i64 )
  State.Run takes-int
  0 ;
export { main };
end;
```

**Why:** `takes-int` asked for an `i64`; `State.Run` is a `State`. The
representation would fit — that is precisely the reasoning the type exists
to forbid. The conversion `as u8` is how a variant *would* cross, and its
current stubbed state is recorded in §3.3.

### Lab 3.6 — Same bits or nothing *(red)*

```tyu
# labs/ch03/red-02-bitcast-width.mod — Lab 3.6 (red)
# bitcast reinterprets bits and requires equal size: i64 is 8 bytes,
# u32 is 4. Expected: rejected at compile time — E3304.
module BitcastWidth;

: shrink ( i64 -- i64 )
  bitcast u32 as i64 ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `bitcast` promises "same bits, new stamp." Eight bytes do not fit
in four, and the compiler declines to silently drop half a value — that
decision is left where it belongs: in `as?`-style code the author writes
deliberately.

### Lab 3.7 — The sealed door *(red)*

```tyu
# labs/ch03/red-03-raw-cast.mod — Lab 3.7 (red)
# Raw pointer casts are compile-time errors — with or without
# langc --allow-raw-casts on the current tree. Expected: E3305.
module RawCast;

: bad ( i64 -- )
  as ptr drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** pointer-to-integer and integer-to-pointer casts break the link
between a value and its type — the link every other check in the language
depends on. They stay errors (E3305), and on the current tree even the
documented `--allow-raw-casts` flag does not open the door. Typed borrows
(chapter 7) are the intended road; the lock is the language taking its own
side.

### Lab 3.8 — A subtype that promises too much *(red)*

```tyu
# labs/ch03/red-04-subtype-output.mod — Lab 3.8 (red)
# Subtype-typed values do not cross word boundaries: a word may take a
# Percent, but it may not promise one. Expected: rejected at compile
# time — E3220 (declared output vs body, which produces the base i64).
module SubtypeOutput;

subtype Percent = i64 range 0 .. 100;

: make-percent ( -- Percent )
  50 as Percent ;

: main ( -- i64 ) make-percent as i64 0 ;
export { main };
end;
```

**Why:** the body produces the base `i64`, the signature promises a
`Percent`, and on the current tree those do not meet — outputs stay in the
base type. The style that survives every version of this rule: convert at
the boundary, in the open, where the check lives.

### Post-mortem — the soup of bits *(optional — for readers with C or embedded scars)*

The C pattern this chapter outlaws one topping at a time: an `int` parameter
that carries motor state, documented as "0 = idle, 1 = run, 2 = fault" in a
header; a status byte whose bits 0, 3, and 5 mean things, packed and
unpacked by hand at four call sites; the "temporary" magic number `42` that
outlives three rewrites. Every one of these compiles forever and documents
itself never. The failure is not the programmer who writes `3` — it is that
the language offered no place to say *3 is not a thing this value can be*.

Tyu's answer is structural: the choice becomes an `enum`, so `3` is not a
`State` and the call site refuses to compile; the range becomes a `subtype`,
so `-1` cannot cross into `take` without passing a check that names the
crime. The soup does not get better comments — it stops being soup.

### From the workbench

Why does `as Percent` trap at run time instead of the checker proving it at
compile time? Because in general it cannot: whether a value is in range is a
fact about *run-time data*, and no paper simulation can know it. What the
checker *can* do — and does — is guarantee the check exists, at the exact
boundary, in the callee, for every caller forever: the same design decision
as chapter 1's `needs` clause. The roadmap item is narrower and worth
waiting for: when a value is a *literal*, the simulation can decide the
range on paper, and the plan is to reject `150 as Percent` at compile time.
The habit to build now is the one that survives: say the conversion, at the
boundary, every time.

### Exercises

1. **A set of your own.** Declare `enum Workday : u8` with `Mon = 1` through
   `Fri = 5`. Write `announce ( Workday -- )`, call it with two variants,
   then write the red-lab variant of Lab 3.5 against it and confirm the
   same rejection.
2. **A range of your own.** Declare `subtype Millivolts = i64 range 0 .. 5000`.
   Write a `calibrate ( Millivolts -- )` word; call it with `3300 as
   Millivolts`. Then predict, before running: what does `6000 as? Millivolts
   swap drop` leave, and what does `6000 as Millivolts` do?
3. **Explain the wall.** Using the words *boundary*, *base type*, and
   *checked conversion*, explain to a colleague why `: make-percent
   ( -- Percent ) 50 as Percent ;` does not compile (Lab 3.8) — and why the
   working alternative is `: make-percent ( -- i64 ) 50 as Percent as i64 ;`.

---

*Next: chapter 4 — Contracts. The promises that are about *relationships*:
`needs` and `ensures`, the shape rules that make them checkable, and the
craft of writing claims the machine reads today and the prover will prove
tomorrow.*
