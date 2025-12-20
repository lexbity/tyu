# Language v1 Reference Manual (Draft)

## 1) Core idea
A small **concatenative, stack-based** systems language for embedded + simulation:
- no GC
- explicit allocation via **regions**
- **fixed arrays** are core (`T'N`)
- higher-level collections/algorithms live in the **stdlib**
- Ada/SPARK-ish safety tools: **subtypes** + **contracts**
- strong MMIO/representation support

**“Profiles”** (debug/rt/baremetal/hosted policies) are enforced by **compiler/build flags**, not a language keyword.

---

## 2) Lexical syntax
**Tokens/punctuation (syntax):**
- `:` start word definition
- `;` end word definition
- `[` `]` quotation
- `{` `}` data literals and typed literals
- `(` `)` stack-effect annotation
- `=>` bind local
- `#` line comment
- `'` fixed-array suffix / index operator (e.g. `u8'16`, `buf'3`, `buf'(i)`)
- `| |` channel type constructor (e.g. `|u32|`)
- `&` / `&!` borrow tokens (core)
- `&[` / `&![` scoped borrow blocks (core)
- `<|` / `|>` channel pipe operators (core syntax sugar)
- `!{...}` quotation effect-set annotation (core)


Numbers: `123`, `-1`, `0xFF`, `0b1010` (optional)
String literal tokens: "..." (see stdlib text/bytes types)

---

## 3) Reserved keywords (minimal)
Reserved words that affect parsing/compilation units:

**Modules / compilation:**
- `module`, `import`, `export`, `end`

**Declarations:**
- `type`, `subtype`, `const`, `resource`, `register-map`, `owned`, `iso`

**Contracts:**
- `requires`, `ensures`

Everything else is a *word* (user or stdlib).

Note: `as`, `as?`, and `bitcast` are conversion operators with dedicated syntax (see §4).

---

# 4) Type system and casting

The language is **statically typed**. Implicit type promotion is **forbidden**. Any conversion must be explicit.

## 4.1 Primitive types

**Unsigned integers:** `u8`, `u16`, `u32`, `u64`, `usize`  
**Signed integers:** `i8`, `i16`, `i32`, `i64`, `isize`  
**Boolean:** `bool` (`true` / `false`)  

**Pointers (typed):**
- `^T`  read-only pointer to `T`
- `^!T` mutable pointer to `T`

Pointers are primarily obtained via the core borrow syntax:
- `&place` produces `^T`
- `&!place` produces `^!T`

## 4.2 Explicit conversions

### `as T` (conversion)
`as` pops the top value and converts it to `T`.

General stack effect:
- `as T : ( x -- y )`

Rules:
- Widening integer conversions are safe (`u8 as u32`, `i16 as i32`).
- Narrowing conversions are explicit and **lossy** (`u32 as u8` truncates high bits).
- Conversions between signed/unsigned are explicit and follow the target’s numeric interpretation.

### `as? T` (checked conversion)
`as?` performs a checked conversion and returns an `ok` flag.

General stack effect:
- `as? T : ( x -- y ok )`

If the conversion cannot be performed without violating the target type’s constraints (e.g., enum tag invalid, subtype range violation), it returns `ok = false`.

### `bitcast T` (bit reinterpretation)
`bitcast` reinterprets the bit pattern without numeric conversion. Sizes must match.

General stack effect:
- `bitcast T : ( x -- y )` with `sizeof(x) == sizeof(y)` required.

Examples:
- `u32 bitcast i32` is allowed
- `u16 bitcast u32` is a compile-time error (size mismatch)

## 4.3 Enums and subtypes in conversions

**Enum → int** via `as` is always valid (returns underlying representation).

**Int → enum**:
- Prefer `as? State` in embedded code; it returns `ok=false` if the integer is not a valid variant tag.

**Subtype conversions** (e.g. `subtype Percent = u8 range 0..100;`):
- `as? Percent` returns `ok=false` if out of range.

Compile-time known constants must be validated at compile time.

## 4.4 Pointer cast policy

All pointer casts are **compile-time errors** unless the compiler is invoked with:

- `--allow-raw-casts`

When enabled, the following become available (still considered low-level/unsafe):
- `^T <-> usize`
- `^T <-> ^U` (retyping/reinterpreting pointers)

Without `--allow-raw-casts`, use `&` / `&!` borrows, `register-map` instances, and typed loads/stores rather than raw address casting.


## 4.5 Structs

`struct` defines a named memory layout (useful for packets, MMIO descriptors, and FFI).

Syntax:

```forth
struct Point
  x : i32
  y : i32
end;
```

### Layout rules
- Default layout is **native** (C-like): fields are aligned and padded by their natural alignment.
- You may add attributes to control layout:
  - `@Packed` (no implicit padding; alignment becomes 1)
  - `@Align(n)` (optional future attribute)

Explicit padding fields are also allowed:

```forth
@Packed
struct PacketHeader
  magic : u16
  len   : u8
  _pad  : u8   # explicit padding / reserved
end;
```

### Field access
There are two related notions: **field values** and **field addresses**.

**Value access**: given a struct value on the stack, `.field` extracts the field:

- `.x : ( Point -- i32 )`

Example:
```forth
p .x p .y +         # sum of coordinates
```

**Address access**: given a pointer to a struct, `->field` computes a pointer to the field:

- `->x : ( ^Point -- ^i32 )`
- `->x : ( ^!Point -- ^!i32 )` (mutable when the input pointer is mutable)

Example:
```forth
pp ->x @i32         # read through pointer
pp ->x 10 swap !i32 # write through mutable pointer
```

**Borrowing fields from places**: if you have a *place* `p` (local/global/resource inside lock), you may also borrow the field place directly:

```forth
&!p.x 10 swap !i32
```

(Recall: `&` / `&!` apply to places, not to arbitrary expressions.)

## 4.6 Enums

`enum` defines a finite set of named variants with a fixed underlying integer representation. Enums are intended for state machines and protocol values.

Syntax:

```forth
enum State : u8
  Idle = 0x00
  Run  = 0x01
  Err  = 0xFF
end;
```

Rules:
- `State.Idle` pushes a value of type `State`.
- Enums do **not** implicitly participate in arithmetic. To do arithmetic, convert explicitly (e.g. `as u8`), then convert back with `as? State`.

Conversions:
- `State -> int` via `as` is always valid (returns underlying value).
- `int -> State` should use `as? State` to avoid traps:

```forth
val as? State  ( State ok )
ok if [ ... ] [ ... ] if
```

The set of valid enum tags is exactly the set of declared variants.


---

## 5) Words and stack execution

### Compiler-recognized intrinsics (control + scoping)

These are *words* with compiler-defined typing/lowering rules (so quotation typing is deterministic).

#### `if`
Syntax:
```forth
cond [ then ] [ else ] if
```

Rule:
- `cond` must be `bool`
- both quotations execute on the **current data stack** and must produce the **same resulting stack types**
- overall effect: `( cond then:Quot( -- R ) else:Quot( -- R ) -- R )` (conceptually)

#### `while`
Syntax:
```forth
[ cond ] [ body ] while
```

Rule (stack must not drift):
- `cond` must have effect `( S -- S bool )` (preserve stack, push `bool`)
- `body` must have effect `( S -- S )`
- overall effect: `( S -- S )`

#### `loop`
Syntax:
```forth
[ body ] loop
```

Rule:
- `body` must have effect `( S -- S )`
- overall effect: `( S -- S )`

#### `lock`
Syntax:
```forth
counter lock [ counter @ 1 + counter ! ]
```

Rule:
- `lock` introduces a scoped capability: inside the quotation, the resource place is writable (`&!counter` and `counter !` are permitted).
- The quotation must be stack-neutral: `( S -- S )` in v1.
- The quotation must be **non-suspending**: effect set must be `{}` (no `suspend`).
- Nested `lock` is forbidden in v1 (compile-time error). Locks are intended for short, atomic critical sections (MMIO/devices/singletons).

#### `return`
`return` exits the current word immediately.

Rule:
- `return` is only allowed in words with an explicit stack effect declaration
- at each `return`, the compiler requires the current stack to match the declared outputs

Words consume/produce stack values:

```forth
10 20 +      # => 30
dup *        # => 900
```

Define words:

```forth
: add3 ( a b c -- sum )
  + +
;
```

Stack effects are v1-optional: documentation first, checkable later.

---

## 6) Locals
`=> name` pops into a local (scoped to the current word).

```forth
: dist2 ( x1 y1 x2 y2 -- d2 )
  => y2 => x2 => y1 => x1
  x2 x1 - dup *
  y2 y1 - dup * +
;
```

Locals are immutable by default in v1.

### Destructuring (value + borrow)

Destructuring is an ergonomic way to unpack a struct into locals.

#### Value destructuring: `=> { x y ... }`

`=> { x y }` pops a struct value and binds locals to its fields (in declared order).

```forth
: move ( Point i32 i32 -- Point )
  => dy => dx
  => { x y }      # pop Point, bind x:i32 y:i32
  x dx +  y dy +  { x y } bitcast Point
;
```

#### Borrow destructuring: `=> { &x &y ... }`

Inside a scoped borrow block (`&[` / `&![`), `=> { &x &y }` computes **pointers to fields** (no loads)
and binds those pointers as locals.

This pairs with generalized scoped borrows (§8.3), so you can borrow any sized value by spilling:

```forth
# Assume p is a Point value on the stack
&[
  => { &x &y }    # x:^i32  y:^i32  (pointers into the spilled temp)
  x @
  y @ +
]
```

- In `&[ ... ]`, field pointers are read-only (`^T`)
- In `&![ ... ]`, field pointers are mutable (`^!T`) when the field is writable


---

## 7) Quotations

### Typed quotations (stack effects)

Quotations are written as `[ ... ]`.

v1 typing rules:

- If a quotation is consumed immediately by a compiler-known intrinsic (e.g. `if`, `while`, `loop`), the compiler checks its stack effect against what that intrinsic requires.
- If a quotation **escapes** (stored, returned, passed to generic `call`, registered as a callback), it must carry an explicit stack effect annotation at the start:

```forth
[ ( a b -- c )  ... ]
```


### Quotation effects (suspension, I/O, etc.)

In addition to a **stack effect**, a quotation may carry an **effect set** when it can perform special control flow (most notably suspension).

Syntax (only required when the quotation *escapes*; optional otherwise):

```forth
[ ( a -- b ) !{suspend}  ... ]
```

Notes:
- Effect sets are a compile-time summary; they do not change runtime representation.
- The compiler can infer effects for immediate quotations (consumed by compiler-known intrinsics), but escaping quotations must be fully annotated: stack effect and (if non-empty) effect set.
- `!{}` is an **annotation token**, not a store operation.

v1 effect names (minimal):
- `suspend` — the quotation may yield/suspend (cooperative scheduling, async-style code)

Rules:
- A `{}` quotation may call any `{}` words/quotations.
- A `{suspend}` quotation may call `{}` or `{suspend}`.
- A `{}` context may not call `{suspend}` unless it uses a handler (see §13).



- `call` requires a typed quotation and is checked at the call site.

v1 closure rule:
- Quotations do **not** implicitly capture outer locals. References inside a quotation must be to globals/module words, or else it is a compile-time error.
`[ ... ]` is code-as-data.

Core calling can be a stdlib word:
- `call : ( quot -- )`

Control flow is preferably via combinators (stdlib):
- `if : ( cond thenQ elseQ -- )`
- `while : ( condQ bodyQ -- )`
- `loop : ( bodyQ -- )`

(These can be stdlib, but they’re “standard”.)

---

# 8) Arrays, slices, vectors

## 7.1 `T'N` is a core language type constructor
Fixed-size inline aggregate.
- `N` must be compile-time constant.
- Stable size/alignment and placement rules.

Typed literals (core):

```forth
const LUT = u16'4{ 10 20 30 40 };
```

You may also allow:

```forth
const LUT : u16'4 = { 10 20 30 40 };
```

### Indexing with `'`

Given an array place `a : T'N`, indexing produces an element **place**:
- `a'k` selects element `k` (0-based). If `a` is addressable, so is `a'k`.
- `&a'k` yields a pointer to the element (`^T` or `^!T` based on mutability).
- Dynamic indices use parentheses: `a'(i)`.

Example:

```forth
=> a
&a'3 @u16
```

## 7.2 `Slice(T)` and `SliceMut(T)` are stdlib ABI types
Recommended ABI shape:

```text
Slice(T)    = struct { ptr:^T,  len:usize }
SliceMut(T) = struct { ptr:^T,  len:usize }   # mutability is in the type, not the pointer token
```

## 7.3 `Vec(T)` is stdlib and region-backed
Dynamic arrays are `owned` and require `Region` for growth/allocation.

---

# 9) Core borrowing syntax

This is the core-language solution to “arrays are values but slices must not dangle”.

## 8.1 Place expressions (what can be borrowed)
A **place** is something with an addressable storage location.

**v1 place grammar:**
```text
place := IDENT ( "." IDENT )*
```

Examples:
- `x` (local)
- `p.x` (field)
- `gpio.OUT_SET` (MMIO register place)
- `counter` (resource name, with rules below)

v1 rule: **no borrowing arbitrary expressions** (no `&(a b +)`), only places.

---

## 8.2 `&place` and `&!place`
Core syntax producing a pointer/reference-like value.

- `&x`  = borrow read-only address of `x`
- `&!x` = borrow mutable address of `x`

These are core tokens (not words).

**Semantics:**
- Produces an address value (typed as `^T` when types exist; otherwise an address + qualifier).
- `&!` is only legal if the place is mutable (local slot, mutable global, locked resource storage, MMIO writeable register, etc.).

**Resources:** if `place` names a `resource`, borrowing it requires you to be inside its lock scope (see §11).

---

## 8.3 `&[ ... ]` and `&![ ... ]` (scoped borrows)

`&[` and `&![` are core scoped-borrow forms that operate on the **top stack value** and then typecheck a block with a temporary borrowed capability.

### Arrays → slices

When the top of stack is `T'N`:

- `&[ block ]` pushes a temporary `Slice(T)` for the duration of `block`
- `&![ block ]` pushes a temporary `SliceMut(T)` for the duration of `block`

**Overall stack effect (array passes through):**
- `&[  block ] : ( S T'N -- S T'N R... )`
- `&![ block ] : ( S T'N -- S T'N R... )`

**Block typing rule:**
At block entry, the compiler pushes the slice on top of the stack. The block must consume it:

- Block entry: `( S T'N Slice(T)    -- ... )`   (or `SliceMut(T)`)
- Block exit:  `( S T'N R...       )`           (slice must be gone)



### Sized values → temporary pointers (generalized borrow)

If the top of stack is any **sized** type `T`:

- `&[ block ]` spills the value to a stable temp and pushes `^T` for the duration of `block`
- `&![ block ]` spills the value to a stable temp and pushes `^!T` for the duration of `block`

This is the general form used for borrow destructuring (§6).

**Block typing rule:**
At block entry, the compiler pushes the temporary pointer on top of the stack. The block must consume it,
and the pointer must not escape the block (see §8.4).
### Regions (borrowed Region handles)

When the top of stack is `Region`:

- `&[ block ]` pushes a temporary **read-only** region handle for the duration of `block`
- `&![ block ]` pushes a temporary **mutable** region handle for the duration of `block`

These borrowed handles are *not* linear themselves; they are scoped capabilities. The owning `Region` value still follows the usual explicit lifecycle (create/destroy).

**Overall stack effect (region passes through):**
- `&[  block ] : ( S Region -- S Region R... )`
- `&![ block ] : ( S Region -- S Region R... )`

### Non-escape rule (compile-time)

Any borrowed value produced by `&[` / `&![` (slice, mutable slice, borrowed region handle) **cannot escape** the block. The compiler rejects:
- returning it from the word
- storing it into globals/resources
- putting it into `Vec` or any `owned` container
- capturing it into an escaping quotation / callback payload
- any other action that would outlive the block

### Suspension rule (compile-time)

At any suspension point (a `suspend` effect boundary, e.g. `yield`), **no borrowed values may be live**:
- no borrowed values on the data stack
- no locals currently bound to borrowed values

Practical consequences (v1 policy):
- `yield` is forbidden inside `lock [ ... ]`.
- `yield` is forbidden inside `&![ ... ]` (mutable borrows).
- `yield` is allowed inside `&[ ... ]` only if the borrowed value is not live at the `yield`.

Example (mutable slice):
```forth
: init-frame ( u8'16 -- u8'16 )
  &![ => s
       s 0 0xAA set
       s 1 0x55 set
     ]
;
```


# 10) Memory model (no GC)

## 9.1 Regions (core concept; API is stdlib/intrinsic)
- `region-create : ( size -- Region )`
- `region-destroy : ( Region -- )`
- `region-alloc : ( Region size -- addr )`

## 9.2 Owned types
`owned` types represent resources / allocations tied to a region or explicit owner rules.

---

# 11) Ada/SPARK-inspired features

## 10.1 Subtypes with ranges
```forth
subtype Percent = u8 range 0..100;
type DutyTable = Percent'16;
```
Constants are checked at compile time; runtime checks are controlled by compiler flags.

## 10.2 Contracts (debug/build-flag controlled)
```forth
: pwm-set ( Percent -- )
  requires [ dup 0 >= swap 100 <= and ]
  => duty
  # ...
;
```
`ensures` can be added later; `requires` alone is already very useful.

---

# 12) Protected/shared resources (portable safety)
```forth
resource counter : u32 = 0 ceiling 5;

: bump ( -- )
  counter lock [
    &!counter @ 1 + &!counter !
  ]
;
```

Rule: a `resource` is only directly addressable/borrowable inside its `lock` block.

(Note: exact concurrency/scheduler model is platform-dependent; `resource` is about safe shared storage access rules.)

---

# 13) MMIO / representation (`register-map`)

`register-map` defines typed, access-checked MMIO registers and (optionally) bitfields and register arrays.
It is a **declaration feature**: it generates addressable register *places* and field accessors that the compiler
can lower to volatile loads/stores with masks/shifts.

## 12.1 Register maps and instances

Declare a register block:

```forth
register-map GPIO
  0x00 OUT     u32 rw volatile
  0x04 OUT_SET u32 wo volatile
  0x08 OUT_CLR u32 wo volatile
  0x20 IN      u32 ro volatile
end;
```

Bind an instance to a base address:

```forth
const gpio = GPIO @ 0x3FF44000;
```

A register access uses a **place**: `gpio.OUT_SET`, `gpio.IN`, etc.

- `&gpio.IN` yields the address of the register (read-only borrow).
- `&!gpio.OUT_SET` yields the address (mutable borrow), only allowed if the register is writeable.

Loads/stores may be typed:

- `@u32`, `!u32`, `@u16`, `!u16`, etc.
- Optionally (if you allow inference for MMIO places), plain `@`/`!` can infer width from the map.

Example:

```forth
&!gpio.OUT_SET (1 13 <<) !u32
&gpio.IN @u32
```

## 12.2 Access modes

Access modes apply both to whole registers and to fields:

- `ro`  read-only (loads ok, stores forbidden)
- `wo`  write-only (stores ok, loads forbidden)
- `rw`  read-write
- `w1c` write-1-to-clear (bit/field semantics; see bitfields)
- `w1s` write-1-to-set
- `rc`  read-to-clear (loads have side effects; discourage RMW)

v1 rule: illegal accesses are compile-time errors when the compiler can see them.

## 12.3 Volatile semantics

If a register is declared `volatile`:
- each read/write must compile to an actual load/store
- the compiler must not remove or merge accesses
- the compiler must not reorder volatile accesses across other volatile accesses

(Keep it conservative for embedded correctness.)

## 12.4 Bitfields

A register entry may include a field block in `{ ... }`.
Field lines use either a single bit (`n`) or a range (`lo..hi`) inclusive.

Syntax:

```forth
register-map UART
  0x00 CTRL u32 rw volatile {
    enable     0        bool rw
    parity_en  1        bool rw
    stop_bits  2..3     u2   rw
    busy       31       bool ro
  }
end;
```

Usage (fields behave like non-addressable places):
- `uart.CTRL.enable @` returns `bool`
- `uart.CTRL.stop_bits 3 !` writes the field

**Important:** bitfields are *not addressable* as raw pointers:
- `&uart.CTRL.enable` is illegal (no real address for a sub-bit field)
- use loads/stores (`@`/`!`) on the field place

### Field read lowering
Reading `uart.CTRL.stop_bits` compiles to:
- volatile load of the parent register
- mask+shift to produce the field value
- optional enum conversion (if field type is enum)

### Field write lowering
For `rw` fields:
- volatile load parent register
- clear mask
- OR shifted value
- volatile store parent register

For `w1c` / `w1s` fields:
- compile to a write that sets only the relevant bits to 1 (no RMW)

For `rc` fields/registers:
- reads may clear status; the compiler may warn on generated RMW writes that would require a read

### Reserved bits
You can declare a reserved field to document and constrain writes:

```forth
0x00 CTRL u32 rw volatile {
  reserved  4..7  u4  ro
  enable    0     bool rw
}
```

`reserved` fields prevent writes to those bits via field access. Whole-register writes are still possible, but can be treated as `unsafe` by convention.

## 12.5 Register arrays

Registers can be declared as arrays by suffixing the type with `'N`:

```forth
register-map GPIO
  0x200 PINCFG u32'40 rw volatile
end;
```

Access uses a place index **without whitespace**:

```forth
&!gpio.PINCFG'13 0x00000002 !u32
```

### Arrays of sub-blocks (nested maps)

You can also repeat a sub-block type:

```forth
register-map TIMER_CH
  0x00 CTRL u32 rw volatile { enable 0 bool rw  busy 31 bool ro }
  0x04 CNT  u32 rw volatile
end;

register-map TIMER
  0x100 CH TIMER_CH'4 volatile
end;
```

Then you can write:

```forth
const tim = TIMER @ 0x3FF50000;
tim.CH'0.CTRL.enable true !
tim.CH'0.CNT @u32
```

**Stride rules**
- For scalar register arrays (`u32`, `u16`, ...), stride is `sizeof(type)` by default.
- For sub-block arrays (`TIMER_CH`), stride is computed as the sub-block size if possible; otherwise you may specify `stride=0x20` as an attribute:

```forth
0x100 CH TIMER_CH'4 volatile stride=0x20
```

## 12.6 Alignment and endianness

- Register offsets must be aligned to their declared width (compile-time error otherwise).
- Endianness is assumed native in v1. If needed later, add `le32/be32`-style types.


# 13) Concurrency model (hybrid): locks for devices, messages for data

Concurrency is **platform-provided** (see Runtime Contract). The language provides:
- an effect system for suspension (`suspend`)
- a minimal ownership story for cross-task data (`iso` + shareable immutable values)
- syntax for channel send/receive (`|>` / `<|`)

## 13.1 Suspension as an effect (no “colored return types”)

A word/quotation may be:
- `{}` — cannot suspend
- `{suspend}` — may suspend (cooperative scheduling)

Suspension is triggered by platform words like `platform.task.yield` and `platform.task.sleep_*`.
From a `{}` context, suspending code can only be invoked through a handler such as `platform.task.run` (platform-defined “run to completion” / “drive scheduler” primitive).

## 13.2 Locks are for singleton devices (no nesting)

`resource lock [ ... ]` is intended for MMIO/devices/singletons:
- lock bodies are atomic, `{}` only, and stack-neutral `( S -- S )`
- nested `lock` is a compile-time error
- suspension inside `lock` is a compile-time error

## 13.3 `iso` vs shareable immutable (“val”)

For cross-task message passing, payload types are classified as:

- **`iso T`** (unique / move-only): can be transferred between tasks without copying.
  - `dup` is illegal on `iso`
  - `drop` is illegal on `iso` unless consumed by an explicit destructor/cleanup word
  - sending an `iso` consumes it

- **shareable immutable** (“val” concept): safe to share freely across tasks.
  - v1 does not introduce a separate `val` keyword; `const` bindings and read-only values (`^T` pointers, immutable structs) are treated as shareable.

## 13.4 Channels and pipe operators

Channels are stdlib/platform types `|T|`.

Operators (core syntax sugar):
- `|>` send: `( |T|  T  -- )`
- `<|` receive: `( |T| -- T )`

Example:
```forth
ch msg |>
ch <| => msg2
```

If `T` is `iso`, `|>` consumes it (move). If `T` is shareable immutable, the program may choose to copy or share by reference (platform/stdlib choice).
