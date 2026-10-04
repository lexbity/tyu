# Chapter 8 — Touching Hardware: MMIO

> The datasheet is a specification.
> Most languages make you retype it as comments.
> Tyu makes you retype it as a contract.
>
> — workshop wall

## 8.1 The Practice

> **The Practice.** *Hardware is described once, in a platform descriptor —
> devices named, registers placed, access rules stated. The source's
> register-map is checked against that description row by row, and every
> access the source writes is checked against the rules. The datasheet
> becomes a compiler input.*

This is the chapter where the book goes to metal. Every lab runs on the
QEMU track (`x86_64-unknown-none`) — the toolchain's maintained hardware
path — under the platform named `x86_64-unknown-none`, whose descriptor
declares an emulated memory aperture holding a handful of devices. QEMU
installation was recommended back in Appendix C precisely for chapters like
this one.

## 8.2 Devices are named, not addressed

Embedded C is full of magic numbers whose meaning lives in a header
somewhere: `#define GPIO_OUT_SET (*(volatile uint32_t*)0x3FF44004)`. The
address is the documentation, the cast is the type, and nothing connects
them to the datasheet but a comment.

Tyu splits the job in two. The **platform descriptor** — a checked-in file
per board — states what exists:

```toml
[[platform.devices]]
map = "Scratch"
instance = "scratch"
registers = [
  { offset = 0x00, name = "A", width = 32, access = "rw" },
  { offset = 0x04, name = "B", width = 32, access = "rw" },
]
```

The **source** declares the register-map it intends to talk to, and binds
it to a named device instance:

```tyu
register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;

const scratch = Scratch @ board.scratch;
```

Then the two are *checked against each other*: row by row, name by name,
width by width, access by access. A source map that misremembers the
hardware does not compile. The `board.scratch` on the right of `@` is a
name resolved against the descriptor — `board.gpio_io` (a real device of a
*different* board in this repo) fails with E3644, Lab 8.3.

And the payoff line, the one worth memorizing: **the source never mentions
an address.** The descriptor owns addresses; the source owns intent.

## 8.3 The access, word by word

With the device bound, registers are places, and chapter 7's borrow-and-
access pattern applies verbatim:

```tyu
&!scratch.A 42 as u32 !u32       # borrow for writing, store a u32
&scratch.A @u32 as i64 42 == check   # borrow for reading, load, compare
```

`&!` borrows the register for writing; `!u32` stores through it. `&`
borrows for reading; `@u32` loads. The `as i64` before `==` is chapter 3's
conversion habit — arithmetic speaks `i64`, hardware speaks its own widths,
and the conversion is where the two meet, visibly.

The `volatile` keyword on each row is a promise the *backend* must keep:
volatile accesses compile to real loads and stores, never elided, never
merged, never reordered across other volatile accesses. This matters more
than almost anything else in the chapter — an optimized-away status poll
is the classic "works with -O0, hangs with -O2" bug — and it is enforced
below the source, where the programmer cannot accidentally defeat it.

Lab 8.1 runs the round-trip on both registers under QEMU.

## 8.4 Write kinds: the hardware's own verbs

Real registers are not all "memory with a funny address." A
write-one-to-clear status register *inverts its meaning under a
read-modify-write*: read it, OR the bit in, write it back — and you have
just cleared every flag that was set. The descriptor states each
register's write behavior, and the lowering obeys:

| Row (descriptor)      | Write behavior                        |
|-----------------------|----------------------------------------|
| `write_kind = "plain"` | store the value                       |
| `write_kind = "w1s"`   | write-1-set: ORs the written bits in  |
| `write_kind = "w1c"`   | write-1-clear: clears the written bits|
| `write_kind = "xor"`   | XORs the written bits in              |
| `read_kind = "effectful"` | reading has side effects (a FIFO pop) |

Lab 8.2 demonstrates all of it, and its most beautiful check is the xor
round-trip: write `0x5`, see `0x5`; write `0x2`, see `0x7`; write `0x2`
*again*, see `0x5` — the register came back where it started, which no
plain store can produce. The w1c section seeds a status register, clears
one bit, and checks the others survived. These are datasheet semantics,
executed and asserted.

Bitfields ride along:

```tyu
0x00 CTRL u32 rw { ctrl_low 0..8 u16 rw }
```

`ctrl_low` is bits 0..8 of CTRL, a `u16` view. Access goes through the
field's own place (`@u16`/`!u16` on `strategy.CTRL.ctrl_low`), with mask
and shift lowered by the backend. What bitfields are *not* is addressable — a
bitfield is a view into a register, not a place of its own, and borrowing
its address is E3608 (Lab 8.6).

## 8.5 The descriptor is the contract

The chapter's red labs are all the same refusal wearing different clothes:
*the source's claim about hardware is checked against the description, and
the description wins.*

- A `board.` name the descriptor does not declare — E3644 (Lab 8.3).
- A map row that disagrees with the descriptor — declare `A` as `ro` where
  the descriptor says `rw` — E3647 (Lab 8.4). The direction of this
  refusal is the whole point: the source does not get to redefine
  hardware by wishful typing.
- A register whose offset misaligns its width — E3611 (Lab 8.5).
- And the foundation of all of it: compiling a module that uses MMIO
  *without a platform at all* — E3640 (Lab 8.7). No descriptor, no
  hardware claims, no program.

## 8.6 The knobs

The QEMU-track commands, as verified in this chapter's labs:

```console
# red labs: compile-time rejection, build against the platform
$ tyu build labs/ch08/red-01-unknown-instance.mod \
      --target=x86_64-unknown-none --platform=x86_64-unknown-none \
      --sysroot=sysroot --out-dir=build/ch8-red
error[E3644]: typecheck error

# green labs: run on the emulated board
$ tyu test --manifest=labs/ch08/manifest.toml \
      --target=x86_64-unknown-none --platform=x86_64-unknown-none \
      --mode=static
    ran=2 ([mmio_strategies_x86,scratch_rt])
test result: ok. 2 passed; 0 failed
```

Two flags are new and both are doing the chapter's job. `--platform` names
the platform pack whose descriptor the source is checked against — forget
it and MMIO does not compile (Lab 8.7's refusal, by design). `--mode=static`
links the fixture image the way this chapter's devices expect; the dynamic
link mode belongs to chapter 13's field-update story and does not yet carry
the aperture relocations.

---

## Labs — Chapter 8

### Lab 8.1 — The scratch round-trip *(green, QEMU)*

```tyu
# labs/ch08/green-01-scratch-roundtrip.mod — Lab 8.1 (green, QEMU)
# The scratch device: two read-write registers in the emulated aperture.
# Write 42 to A, read it back, then B. The device is NAMED in the
# platform descriptor; the source never mentions an address.
# Expected: under QEMU (x86_64-unknown-none), all checks green, S marker.
module ScratchRT;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;

const scratch = Scratch @ board.scratch;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: scratch-rt-run ( -- )
  &!scratch.A 42 as u32 !u32
  &scratch.A @u32 as i64 42 == check
  &!scratch.B 100 as u32 !u32
  &scratch.B @u32 as i64 100 == check ;

export { scratch-rt-run };
end;
```

Run it with the chapter manifest (§8.6). Note what is *absent*: no
addresses, no casts to pointer types, no `volatile` folklore to remember —
the register is a place, and places are chapter 7's territory.

### Lab 8.2 — The hardware's own verbs *(green, QEMU)*

Fixture: `labs/ch08/green-02-strategies.mod` — a longer exercise over the
`strategy` device, asserted end to end:

- **w1s**: write `0x5` to `SETBITS`, read `0x5`; write `0x2`, read `0x7`;
  write `0x0`, still read `0x7` (write-zero sets nothing).
- **w1c**: seed `STATUS` with `0x7` through a plain write, clear `0x4` with
  a write-one-to-clear, read `0x3` — the *other* bits survived.
- **xor**: `0x5`, then `0x2` (reads `0x7`), then `0x2` again — back to
  `0x5`. Idempotence from an inverting register, produced by the correct
  read-modify-write lowering.
- A **bitfield** round-trip on `CTRL.ctrl_low`, bits 0..8 of a `u32`,
  accessed as a `u16`.

The descriptor's `write_kind` column is quoted in §8.4; the lab is that
column, running.

### Lab 8.3 — A device from another board *(red)*

```tyu
# labs/ch08/red-01-unknown-instance.mod — Lab 8.3 (red)
# board.gpio_io does not exist in THIS platform's descriptor: devices are
# named per board, and the checker resolves the name against the
# descriptor. Expected: rejected at compile time — E3644.
module UnknownInstance;

register-map GPIO
  0x00 OUT_SET u32 wo volatile
  0x20 IN      u32 ro volatile
end;

const gpio = GPIO @ board.gpio_io ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** `gpio_io` is a real device — of the hosted board, a different
descriptor. Names resolve against *this* platform's description, and this
platform does not have that device. The register-map's shape matched
perfectly; the match was to the wrong datasheet.

### Lab 8.4 — The wishful access mode *(red)*

```tyu
# labs/ch08/red-02-row-mismatch.mod — Lab 8.4 (red)
# The map declares A as ro; the descriptor says rw. The descriptor wins:
# rows must match, name by name, width by width, access by access.
# Expected: rejected at compile time — E3647.
module RowMismatch;

register-map Scratch
  0x00 A u32 ro volatile
  0x04 B u32 rw volatile
end;

const scratch = Scratch @ board.scratch ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** declaring a register `ro` in source cannot *make* it read-only —
only the hardware (via the descriptor) can be right about itself. Every
field of every row is checked; the source's datasheet transcription is a
claim, and E3647 is its proof obligation.

### Lab 8.5 — The misaligned register *(red)*

```tyu
# labs/ch08/red-03-misaligned.mod — Lab 8.5 (red)
# A 32-bit register at offset 0x01: hardware would tolerate it; the
# checker will not. Expected: rejected at compile time — E3611.
module Misaligned;

register-map Bad
  0x01 A u32 rw volatile
end;

const bad = Bad @ board.scratch ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** a `u32` register at offset `0x01` is either a transcription error
or a bus fault waiting; the checker rejects it at the declaration, before
any access exists. Alignment is the cheapest lie to catch and the most
common one typed.

### Lab 8.6 — A bitfield is not a place *(red)*

```tyu
# labs/ch08/red-04-bitfield-addressable.mod — Lab 8.6 (red)
# A bitfield is a view into a register, not a place: borrowing its
# address is illegal. Expected: rejected at compile time — E3608.
module BitfieldAddressable;

register-map Strategy
  0x00 CTRL u32 rw { ctrl_low 0..8 u16 rw }
end;

const strategy = Strategy @ board.strategy ;

: f ( -- ) &strategy.CTRL.ctrl_low @u16 drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** bits 0..8 of CTRL have no address — a borrow needs a place, and a
view into a register is not one. Field access goes through the register
with the typed field words; the refusal keeps "places" (chapter 7) an
exactly-defined set.

### Lab 8.7 — No descriptor, no hardware *(red)*

```tyu
# labs/ch08/red-05-needs-descriptor.mod — Lab 8.7 (red)
# MMIO without a platform descriptor cannot be checked, so it cannot be
# compiled: build this module without --platform and the compiler refuses.
# Expected: rejected at compile time — E3640.
module NeedsDescriptor;

register-map Scratch
  0x00 A u32 rw volatile
end;

const scratch = Scratch @ board.scratch ;

: main ( -- i64 ) 0 ;
export { main };
end;
```

**Why:** build it *without* `--platform` and the compiler refuses (E3640):
`board.scratch` cannot resolve against a description that was not given,
and unchecked hardware access is not a degraded mode — it is no mode. The
refusal is the chapter in miniature.

### Post-mortem — the read-modify-write that cleared the flags *(optional — for readers with C or embedded scars)*

The C original: a status register documented as "write 1 to clear," read
into a local, a bit OR-ed in, written back — and every flag that was set
at read time silently cleared, because writing back the read value wrote
ones into all of them. The bug needs no typo, no race, no exotic path: any
innocent read-modify-write does it, and the datasheet warned in one line
of fine print. Its siblings — the FIFO whose read *pops*, the register
whose write *sets* — differ only in the flavor of the surprise.

The descriptor's `write_kind`/`read_kind` columns make the fine print
machine-read. Given `w1c`, the lowering emits a write that clears exactly
the written bits; given `w1s`, one that sets them; given an `effectful`
read, the checker can treat the read itself as an event. The programmer
does not remember the rule — the register *is* the rule, from declaration
through lowering to the QEMU run that asserts the XOR round-trip no plain
store could pass.

### From the workbench

Why bind devices by *name* instead of letting the source carry addresses?
Because the address is the part most likely to be wrong and least likely
to be checked. A wrong address in C is a valid program that corrupts
whatever lives at that address — often nothing, often something vital. A
wrong name in Tyu is E3644; a wrong row is E3647; a wrong offset is
E3611. Every one of those errors used to be a hardware bug found with an
oscilloscope, and is now a sentence from the compiler with the descriptor
as its citation. The descriptor also belongs to the platform pack, not to
the program — so two programs on one board cannot disagree about the
board, and one board's truths are checked once, centrally, where a fix
reaches everyone.

### Exercises

1. **Add a register.** Extend Lab 8.1: declare row `0x08 C u32 rw` in the
   source map only. Compile against the descriptor, meet the mismatch
   error, and then fix the fact, not the claim — by extending the *descriptor's*
   `Scratch` device with row C and re-running. Which of the two edits is
   the claim, and which is the fact?
2. **Round-trip a bitfield.** Using Lab 8.2's `CTRL.ctrl_low`, write
   `0xA5`, read back, and check the upper bits of CTRL never moved. (The
   lowering's mask-and-shift is the promise; this check is its proof.)
3. **Explain the direction.** Lab 8.4's map and the descriptor disagree
   about `A`'s access mode. In two sentences: why must the descriptor win,
   and what would go wrong on real hardware if the source's `ro` claim
   were believed over the datasheet's `rw`?

---

*Next: chapter 9 — Factoring: Style Becomes Proof. The Thinking Forth
craft, kept accountable by the checker: word granularity, the factoring
criteria, and the Tyu claim that how you divide a word changes what the
checker can prove about it.*
