# Chapter 12 — Tasks and Channels: Concurrency the Platform Provides

> The scheduler is not in the language.
> The language is what makes the scheduler safe to use.
>
> — workshop wall

## 12.1 The Practice

> **The Practice.** *Concurrency is declared, not assumed: the platform
> supplies the scheduler, the effects say who may suspend, the annotations
> say what escapes into it, and every safety property it needs — locks,
> custody, bounds — is one this book has already paid for.*

This chapter is short, and the shortness is the argument. Chapters 5 and 7
already built everything concurrency safety needs: `suspend` is an effect
with a discharging handler, shared storage is a resource with a lexically
granted lock, and a message is custody that moves. What remains is the
platform's scheduler — and the two places where the compiler holds the
new pieces to the old rules.

## 12.2 The platform supplies the scheduler

Tyu mandates no scheduler. The word `platform.task.run`, the spawn and
join machinery, the sleep clock — all of it belongs to the platform's
sysroot, present where the hardware and runtime provide it and absent
where they do not. The hosted platform (this chapter's home) ships the
full set: a four-worker work-stealing scheduler lives in its runtime.
The bare-metal QEMU board carries task machinery too — and *no*
`platform.channel` at all. That asymmetry is not an inconsistency; it is
the platform interface telling the truth, and Lab 12.6 shows the compiler
enforcing it: import `platform.channel` in a program built for the board
and the build fails with E2201 — the interface file does not exist for
that platform, because the capability does not exist on that platform.
**The platform's interface is a contract about what the hardware
provides**, the same lesson chapter 8 taught about devices, now applied
to concurrency.

`run` remains the pivotal word from chapter 5: the handler whose
quotation *discharges* `suspend`. Everything in this chapter happens
inside one.

## 12.3 spawn and join: the grammar of escaping quotations

A worker task is a quotation handed to the scheduler — and the moment a
quotation is handed to anything rather than consumed by `if`/`while`, it
becomes an *escaping* quotation, and the grammar from chapter 4's shape
rules applies one more time: it must state its own interface.

```tyu
[ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ]
platform.task.spawn
```

The `( -- )` annotation is the quotation's stack effect — stack-neutral —
and without it the spawn is rejected (E3760, Lab 12.4). The rule is
chapter 4's, transposed: a contract must state its shape because the
scheduler will hold it to that shape from a context the author cannot
see. The annotation is the handoff's receipt.

`spawn` returns a `Task` — a handle, and like chapter 7's channel handle,
a copyable doorway rather than the custody itself. `join` consumes the
handle and performs `suspend`: it may block until the worker finishes.
And there is the trap that teaches the discharge rule twice over — `join`
in `main` is E5001 (Lab 12.5), because suspension needs a handler; the
spawn-and-join pair therefore lives inside `run`'s quotation, where the
suspendable grant is active and the suspend discharges. Lab 12.1 is the
whole pattern, verified end to end: spawn a worker that locks the shared
counter and increments it, join, and only then lock and check.

## 12.4 Two workers, one total

Lab 12.2 spawns the worker twice and joins both:

```tyu
[ ( -- )
  [ ( -- ) Counter lock [ … ] ] platform.task.spawn platform.task.join
  [ ( -- ) Counter lock [ … ] ] platform.task.spawn platform.task.join
] platform.task.run
Counter lock [ &!Counter @i64 2 == check ]
```

The execution is genuinely concurrent — two tasks, a work-stealing
scheduler, real parallel interleavings on multi-core hosts — and the
result is deterministic *by construction*: both workers are joined before
the check, and both increments went through the chapter 11 lock. That
sentence deserves unpacking, because it is the chapter's whole thesis in
one line: **no new safety mechanism appears in this chapter.** The lock
is chapter 5's grant and chapter 11's cross-context rule; the check is
chapter 2's `check`; the determinism comes from join, whose cost is a
declared `suspend` — visible in the interface, discharged by the visible
handler. Concurrency safety in Tyu is not a chapter of new rules; it is
the old rules observed from a second context.

## 12.5 Sleeping

`platform.task.sleep-ms` completes the suspend family, and its usage
shows the conversion habit one more time — the duration crosses the word
boundary as a `usize`:

```tyu
[ ( -- )
  "napping\n" platform.io.log
  1 as usize platform.task.sleep-ms
  "awake\n" platform.io.log
] platform.task.run
```

Sleep, like yield and join, performs `suspend`; it runs inside the same
discharging handler; and the check of Lab 12.3 is the same silence that
has marked success since chapter 1. (The sleeps here are milliseconds —
note the argument *before* the word, stack order, and the `as usize`.)

## 12.6 Custody at a distance

Chapter 7's channels were introduced as doorways for custody: `send`
consumes, `recv` receives, and between them the value exists in exactly
one place the checker can name. For `i64` payloads that is a convenient
transfer; the design's real target is `iso` payloads — messages whose
whole meaning is sole ownership (a buffer being donated, a capability
being delegated). The channel's `send` is compiler-owned *because* of
that: the move semantics are not the sysroot's to get wrong. The hosted
channel today is `|i64|`-typed; the general `iso`-across-channel story is
the composition of chapter 7's linearity with this chapter's handoff —
and like the native-stack proof, its full form is on the roadmap, with
every ingredient already checked.

---

## Labs — Chapter 12

### Lab 12.1 — Spawn, join, check *(green)*

```tyu
# labs/ch12/green-01-spawn-join-run.mod — Lab 12.1 (green)
# The full pattern: spawn a worker (its quotation escapes, so it carries
# an annotation), join it, and only then check the shared resource.
# join performs suspend, so the pair lives inside a run handler.
# Expected: silent clean run (exit 0, marker emitted, no F).
module SpawnJoin;
import platform/linux { platform.io.log platform.task.spawn platform.task.join platform.task.run };

resource Counter : i64;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  [ ( -- )
    [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn
    platform.task.join
  ] platform.task.run
  Counter lock [ &!Counter @i64 1 == check ]
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean, then count the *contexts* the counter's rule had to
satisfy: the worker's lock, main's post-join lock — and note which one
the cross-context rule of chapter 11 would demand even without
concurrency.

### Lab 12.2 — Two workers, one total *(green)*

```tyu
# labs/ch12/green-02-two-workers.mod — Lab 12.2 (green)
# Two workers increment the same counter; both are joined before the
# check. The result is deterministic even though the execution is not.
# Expected: silent clean run (exit 0, marker emitted, no F).
module TwoWorkers;
import platform/linux { platform.io.log platform.task.spawn platform.task.join platform.task.run };

resource Counter : i64;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  [ ( -- )
    [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn platform.task.join
    [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn platform.task.join
  ] platform.task.run
  Counter lock [ &!Counter @i64 2 == check ]
  "S\n" platform.io.log 0 ;
export { main };
end;
```

Run it clean. Then move the check *inside* the run quotation, before the
second spawn — run it ten times and consider what the check is now
racing against, and why the joined version has no such race to lose.

### Lab 12.3 — The nap *(green)*

```tyu
# labs/ch12/green-03-sleep.mod — Lab 12.3 (green)
# sleep belongs to the same suspend family as yield: it performs
# {suspend} and runs inside the run handler that discharges it.
# Expected: prints napping, then awake, then the marker; exits 0.
module Nap;
import platform/linux { platform.io.log platform.task.run platform.task.sleep-ms };

: main ( -- i64 )
  [ ( -- ) "napping\n" platform.io.log 1 as usize platform.task.sleep-ms "awake\n" platform.io.log ] platform.task.run
  "S\n" platform.io.log 0 ;
export { main };
end;
```

### Lab 12.4 — The handoff without a receipt *(red)*

```tyu
# labs/ch12/red-01-unannotated-quotation.mod — Lab 12.4 (red)
# spawn's quotation escapes into the task system: it must carry its
# stack-effect annotation. Expected: rejected at compile time — E3760.
module Unannotated;
import platform/linux { platform.task.spawn platform.task.join platform.task.run };

: main ( -- i64 )
  [ ( -- )
    [ 1 2 + ] platform.task.spawn
    platform.task.join
  ] platform.task.run
  0 ;
export { main };
end;
```

**Why:** a quotation consumed by `if` is checked in place; a quotation
handed to `spawn` will run in a context the author does not write. The
annotation `( -- )` is the escaping code's contract — the shape the
scheduler holds it to.

### Lab 12.5 — Joining with nothing to join under *(red)*

```tyu
# labs/ch12/red-02-join-outside-handler.mod — Lab 12.5 (red)
# join performs suspend; main is not a suspendable context, so the join
# has nothing to discharge it. Expected: E5001.
module JoinOutside;
import platform/linux { platform.task.spawn platform.task.join };

resource Counter : i64;

: main ( -- i64 )
  [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn
  platform.task.join
  0 ;
export { main };
end;
```

**Why:** `join` performs `suspend` — waiting is suspension — and `main`
grants no `suspendable`. The spawn-and-join pair belongs inside `run`'s
discharge, which is exactly where Lab 12.1 puts it.

### Lab 12.6 — The channel the board does not have *(red, cross-platform)*

```tyu
# labs/ch12/red-03-channel-absent.mod — Lab 12.6 (red)
# platform.channel exists only in the hosted platform's interface. Build
# this for the board (x86_64-unknown-none) and the import fails: the
# platform's interface is the contract. Expected: E2201.
module ChannelAbsent;
import platform/channel;

: f ( -- |i64| ) platform.channel.make ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

```console
$ langc --emit=tc labs/ch12/red-03-channel-absent.mod \
        --target=x86_64-unknown-none --sysroot=sysroot
error[E2201]: import interface file (.def) not found
```

**Why:** the board's sysroot ships no `platform.channel` — the hardware
path has no channel runtime — and the import resolution reads the
platform's interface inventory as the contract. The same source compiles
hosted and fails for the board: that is not a toolchain inconsistency,
it is the platform asymmetry made into a compile-time fact.

### Post-mortem — the thread that shared a bool *(optional — for readers with C or embedded scars)*

The C pattern this chapter refuses: two threads, one shared `int`, a
comment saying "reads are atomic, mostly," a mutex added on the write
path only — and the program that passes every test until the optimizer
reorders a load the comment said nothing about. The genre's signature is
that the safety mechanism and the safety requirement live in different
places — the requirement in the data flow, the mechanism in whatever
locking the author remembered.

Tyu's arrangement is the inversion. The requirement is visible: the
counter is a `resource`, so the lock is not optional (chapter 11's
E5031 would fire on the unlocked path *even with no threads at all*).
The handoff is visible: `send` moves, and an `iso` payload cannot be
duplicated on the way (chapter 7). The suspension is visible: join and
yield declare their `suspend`, and the discharge sits in a lexical
bracket. A concurrency bug in this language has to be an inconsistency
between checked claims — which is the politest possible description of
"rare."

### From the workbench

Why is the concurrency chapter the shortest in the book? Because the
design pushed every hard decision earlier, where each could be checked in
isolation. Suspension became an effect in chapter 5 so that it would not
need a new analysis here. Custody became a type property in chapter 7 so
that message passing would not need a new analysis here. The scheduler
itself was pushed out of the language entirely — the platform provides
it, the sysroot declares it, and the import resolution (E2201) keeps
programs aligned with what their board actually has. A short chapter
about a hard subject is not an omission; it is a receipt for work already
done.

### Exercises

1. **Three workers.** Extend Lab 12.2 to three spawns and joins, predict
   the final counter, and confirm. Then delete one `join` and predict
   *precisely* which check fails first — the compile-time E-code or the
   runtime check — before running.
2. **The annotated effect.** Change a worker quotation's annotation from
   `( -- )` to `( -- i64 )` while keeping its body stack-neutral. Which
   check rejects it, where, and what does that rejection protect the
   *scheduler* from?
3. **The asymmetric platform.** A teammate wants channels on the QEMU
   board "because the code compiles on hosted." Using the words
   *interface*, *platform contract*, and *runtime capability*, write the
   two-sentence answer — and name the chapter-8 mechanism that plays the
   same role for hardware.

---

*Next: chapter 13 — Field Updates on Untrusted Media. The capstone: the
`.lmod` container, signing, trust tiers, the `abi_hash` gate — and the
whole book's discipline concentrated into one daring, checked operation.*
