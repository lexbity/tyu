# Compiler Technical Specification v1 (Draft)

This document specifies the **v1 compiler engineering design** for the language, covering:
- front-end (parsing, name resolution, typing)
- mid-end (typed stack IR + safety checks)
- backends: **native AOT** and **bytecode**
- attributes and compiler-recognized intrinsics
- quotation typing model (clarified)

Bytecode VM/runtime details are intentionally deferred.

---

## 1) Design decisions locked

1. **Core forms are compiler-recognized intrinsics.**  
   Surface syntax stays “word-like”, but control flow + a few special operators are recognized by the compiler.

2. **Locals are immutable only.**  
   `=> name` binds once; no reassignment in v1.

3. **Subtype + contract checks are inserted by default** (unless explicitly disabled by flags).  
   The point is to prevent embedded bugs; checks are “on” by default.

4. **Quotations are statically typed where it matters.**  
   Inference is allowed for immediate-use quotations; explicit stack effects are required for escaping quotations.

---

## 2) Tooling / artifacts

### 2.1 Compiler driver (`langc`)
Minimum flags:
- `--emit=obj|asm|bc|ir`
- `--target=<triple>`
- `-O0..-O3`
- `-g`
- `--checks=off|contracts|all` (default: `all`)
- `--allow-raw-casts` (default: off)
- `--sysroot=<path>`
- `-I <path>` include paths
- `--out-dir=<path>`

Outputs:
- native AOT: `.o` (ELF object), optionally `.elf` if the driver also links
- bytecode: `.lbc` module container (format defined later)
- debug dumps: `.lir` / `.json` optional for tooling

---

## 3) Compilation units and module model

### 3.1 Units
- A compilation unit is a single `module Name; ... end;`
- Optional interface file: `Name.def`
- Implementation file: `Name.mod`

### 3.2 Interface/implementation checking
If `Name.def` exists, `Name.mod` must match:
- exported symbol names and kinds (word/type/const/register-map/etc.)
- signatures / stack effects for exported words
- ABI-relevant attributes (e.g. `@ISR`, `@CAbi`, `@ExportName`)
- (optional) exported contracts as documentation

### 3.3 Imports
`import M { a b c }` binds symbols from module `M`.
Resolution order is controlled by:
- package/workspace roots
- dependency graph
- sysroot (`core`, platform packages)

---

## 4) Front-end pipeline

### 4.1 Lexing
Token types:
- identifiers (“words”)
- numbers
- strings
- punctuation: `{ } [ ] ( ) : ; => . .. <| |>`  (multi-char tokens use longest-match)
- effect-set annotation: a single token of the form `!{name[,name...]}` (only valid after a stack-effect annotation in an escaping quotation)
- attributes: tokens beginning with `@Name` (only valid in declaration position)
- comments: `# ... end-of-line`

### 4.2 Parsing to AST
AST constructs:
- declarations: `type`, `subtype`, `struct`, `enum`, `const`, `resource`, `register-map`, `word`
- word: attributes, name, optional stack effect `( ... -- ... )`, optional `requires`/`ensures`, body terms
- terms:
  - literals
  - word references
  - quotations `[ ... ]`
  - operators: `as T`, `as? T`, `bitcast T`
  - place borrows: `&place`, `&!place`
  - scoped borrows: `&[ ... ]` / `&![ ... ]` (Arrays → Slice views; Regions → borrowed Region handles; any sized `T` → spilled temp + `^T`/`^!T`)
  - channel pipes: `<|` / `|>` (syntax sugar; lower to stdlib/sysroot channel ops)

### 4.3 Name resolution
Resolve:
- word calls
- type names
- struct field accessors (`.field`, `->field`)
- enum variants (`State.Idle`)
- register-map paths (`gpio.CTRL.MODE`, `gpio.PINCFG'13`)

---

## 5) Type checking and static semantics

### 5.0 Type grammar & precedence (pinned for v1)

This block is **normative** for parsing/typing constructed types.

#### Tokens used in types
- Pointer types: `^T` (read-only), `^!T` (mutable)
- Fixed-size arrays: `T'N` where `N` is a compile-time constant integer
- Channels: `|T|` (type constructor; implementation lives in the sysroot)

#### Grammar (EBNF-ish)

```
Type        ::= Qual* TypePref
Qual        ::= "iso" | "owned" | "resource" | "lock"        # v1 qualifiers

TypePref    ::= PtrPref* TypePost
PtrPref     ::= "^" | "^!"

TypePost    ::= TypePrim ArraySuf*
TypePrim    ::= Ident
             | "(" Type ")"
             | ChanType

ChanType    ::= "|" Type "|"
ArraySuf    ::= "'" ConstInt
ConstInt    ::= /[0-9]+/
```

#### Precedence rules (no surprises)

Postfix array suffix binds tighter than pointer prefix:

- `^u8'16` parses as `^(u8'16)`  *(pointer to array)*
- `(^u8)'16` parses as an array of pointers  *(must be explicit)*
- `iso ^u8'16` parses as `iso ^(u8'16)`  *(qualifiers are outermost)*

The compiler should reject confusing forms without parentheses if it cannot be parsed unambiguously.

### 5.1 Data layout & field offsets (normative)

This section defines `sizeof(T)`, `alignof(T)`, and `off(field)` for **native** structs and arrays. These values are compile-time constants used by:
- field address computation (`->field`, borrow destructuring `=> { &f ... }`)
- array indexing (`place'idx`)
- stack slot/temporary sizing for spills

#### 5.1.1 Primitive sizes and alignment

For a given target profile:
- `u8/i8/bool` : size 1, align 1
- `u16/i16`    : size 2, align 2
- `u32/i32`    : size 4, align 4
- `u64/i64`    : size 8, align 8
- `usize/isize`: size = pointer size, align = pointer alignment
- `^T` / `^!T`: size = pointer size, align = pointer alignment

Endianness does **not** affect byte offsets (it only affects how multi-byte scalars are interpreted by loads/stores).

#### 5.1.2 Arrays

For `A = T'N`:
- `alignof(A) = alignof(T)`
- `sizeof(A)  = N * sizeof(T)`
- element `i` address: `addr(A) + i * sizeof(T)` (see §10.2)

#### 5.1.3 Struct layout (default native)

Unless annotated otherwise, `struct S { f1:T1, f2:T2, ... }` uses *native (C-like)* layout:

Algorithm:
1. `off = 0`, `struct_align = 1`
2. For each field `fi:Ti` in declaration order:
   - `a = alignof(Ti)`
   - `off = round_up(off, a)`
   - `off(fi) = off`
   - `off += sizeof(Ti)`
   - `struct_align = max(struct_align, a)`
3. `sizeof(S) = round_up(off, struct_align)`
4. `alignof(S) = struct_align`

The compiler computes `off(fi)` at compile time. Field offsets are stable for a given target profile and set of layout attributes.

#### 5.1.4 Layout attributes

If the language manual allows attributes (e.g., for packets/FFI), the compiler uses:

- `@Packed` on a `struct`:
  - `alignof(S) = 1`
  - no implicit padding between fields:
    - `off(f1)=0`
    - `off(fi+1)=off(fi)+sizeof(Ti)`
  - `sizeof(S) = sum(sizeof(Ti))`
  - Note: field loads/stores may be unaligned; the backend must either emit unaligned accesses when supported or lower to bytewise ops.

- `@Align(n)` (future / optional):
  - `alignof(S) = max(native_alignof(S), n)`
  - `sizeof(S)` is rounded up to `alignof(S)`.

If multiple attributes conflict, the compiler must reject the program (v1: keep it simple).

#### 5.1.5 Bitfields in register-map

For a register-map field `name bit width`, bits are numbered with **LSB = 0** on the integer value produced by the volatile load.
- `mask = ((1 << width) - 1) << bit`
- `read = (reg >> bit) & ((1<<width)-1)`
- `write`: read-modify-write with masked update unless the register is declared `wo` or the field is write-one-to-set/clear (sysroot may provide specialized ops).

Bit numbering is defined on the **value**, so it remains consistent across endianness; endianness only affects how the underlying bytes are assembled into the integer by `@u32`, etc.

### 5.2 Typed stack checking
The typechecker maintains a stack of types per word body.
- literals push types
- word calls pop/push by signature
- `=> name` pops and binds an immutable local
- local reference pushes its type

If a word declares `( ... -- ... )`, it must match computed effect.


Move-only / uniqueness (for `iso`):
- The type system may tag a type as `iso T` (unique ownership, move-only).
- Enforce at compile time for the following core stack ops (or their stdlib equivalents):
  - `dup` is illegal if the top value is `iso _`
  - `drop` is illegal if the dropped value is `iso _` (must be consumed by an explicit destructor/cleanup op)
- Channel send (`|>`) consumes its payload; if the payload is `iso T`, it is moved to the receiver.

Shareable immutable (“val” concept):
- v1 does not require a dedicated `val` keyword. The compiler may treat immutable values (including `const` data and read-only pointers) as shareable across tasks.

### 5.3 Subtypes (checks inserted by default)
Default insertion points (v1):
- on `as? Subtype` conversions (required)
- on `as Subtype` conversions (policy: either trap-on-fail, or lower to `as?` + trap)
- on word boundaries for parameters/returns that are subtypes (required by default)

Compile-time constants must be validated at compile time.

### 5.4 Contracts (checks inserted by default)
When enabled:
- evaluate `requires` predicates at entry and `ensures` predicates at exit
- if false: trap

Under `@ISR`, contract predicates must be ISR-safe:
- no allocation
- no blocking
- only call ISR-safe words (flag-propagated)

### 5.5 Casting
- `as T` conversion
- `as? T` checked conversion → pushes `ok:bool`
- `bitcast T` same-size reinterpret

Pointer casts are compile-time errors unless `--allow-raw-casts`.

---


### 5.6 Locals, bindings, and destructuring (value + borrow)

#### `=> name` (binding)
`=> name` pops the top value and binds an immutable local `name : T`.

Typing rule:
- Stack before: `... T`
- Stack after:  `...`
- Env after: `name : T`

#### `=> { f1 f2 ... }` (value destructuring)
Value destructuring **moves** a struct value into locals for its fields.

Typing rule:
- Stack before: `... S`
- where `S` is a struct with fields `{ f1:T1, f2:T2, ... }` (in declared order)
- Stack after:  `...`
- Env after: `f1:T1, f2:T2, ...`

Lowering (conceptual):
- Equivalent to extracting each field from the consumed value and binding locals.
- The original struct value is considered moved.

#### `=> { &f1 &f2 ... }` (borrow destructuring)
Borrow destructuring is only valid when the top of stack is a **pointer to a struct**:
- `^S` inside `&[ ... ]`
- `^!S` inside `&![ ... ]`

It computes **field pointers** (no loads) and binds them as locals.

Typing rule:
- Stack before: `... ^S` (or `... ^!S`)
- Stack after:  `...`
- Env after:
  - if base is `^S`: `f1:^T1, f2:^T2, ...`
  - if base is `^!S`: `f1:^!T1, f2:^!T2, ...` (subject to field writability rules)

Lowering (conceptual):
- For each field `fi` with byte offset `off(fi)`:
  - `ptr_fi = base_ptr + off(fi)`
  - bind `fi` to `ptr_fi` (typed as `^Ti` or `^!Ti`)
- The base pointer is consumed by the destructuring operation (like normal `=>`).

This feature is designed to pair with **generalized scoped borrows** (see §8.3 Case C),
so you can do:

```forth
&[
  => { &x &y }   # x,y are pointers into the spilled temp
  x @           # load field x
  y @ +         # load field y
]
```
## 6) Quotations: typing model (clarified)

Quotations: `[ ... ]`.

Two usages:
1) **Immediate**: consumed by compiler-known intrinsics with known quotation signature.
2) **Escaping**: stored/returned/passed to generic `call` or registered as callback.

v1 rules:
- Immediate quotations: effect is inferred/checked in context.
- Escaping quotations: must include an explicit **stack effect** annotation at the start, and may optionally include an **effect set** annotation immediately after it.

```forth
[ ( a b -- c )  ... ]
[ ( a b -- c ) !{suspend}  ... ]
```


Effect sets:
- Each word/quotation is assigned an effect set (default `{}`).
- `suspend` is the only required v1 effect (triggered by `platform.task.yield`, `platform.task.sleep_*`, and any other sysroot word marked suspending).

Propagation (summary):
- A body that calls a `{suspend}` callee is `{suspend}`.
- `{}` may call `{}` freely.
- `{suspend}` may call `{}` or `{suspend}`.
- A `{}` context may not call `{suspend}` unless the call is wrapped by a handler word (platform-defined), e.g. `platform.task.run`.

Static restriction:
- At any suspension point, no scoped-borrow values may be live (see §8).

`call` requires a typed quotation and is checked at the call site.

v1 closure rule:
- quotations do **not** implicitly capture outer locals.
- references inside quotations must be to globals/module words; local capture is a compile-time error.

---

## 7) Compiler-recognized intrinsics (core control)

### Intrinsic typing rules (pinned for v1)

These intrinsics are recognized by name and have compiler-defined typing/lowering. They are the basis for quotation inference.

- **`if`**
  - surface: `cond [then] [else] if`
  - type rule: `cond:bool`; both quotations are checked from the same entry stack-state and must yield identical exit stack-state.

- **`while`**
  - surface: `[cond] [body] while`
  - type rule: `cond : ( S -- S bool )`, `body : ( S -- S )`, overall `( S -- S )`.

- **`loop`**
  - surface: `[body] loop`
  - type rule: `body : ( S -- S )`, overall `( S -- S )`.

- **`lock`**
  - surface: `resource lock [ ... ]`
  - type rule: quotation must be `( S -- S )` and effect `{}`.
  - nested `lock` is forbidden in v1 (compile-time error).
  - within the quotation, `resource` is treated as a writable place (scoped capability).

- **`return`**
  - terminator intrinsic
  - allowed only when the enclosing word has an explicit stack-effect declaration; at each `return` the current stack must match the declared outputs.


Suggested v1 intrinsic set:

Control flow:
- `if`
- `while`
- `loop`
- `return` (optional)

Borrow/slice blocks:
- `&[` / `&![` scoped borrow block forms are intrinsic syntax.

Resource helpers (optional intrinsics for safety):
- `with-region`
- `lock`

---


### Channel pipe operators (`<|` / `|>`)

`<|` and `|>` are parsed as dedicated operator terms and lowered to **sysroot-defined** channel operations.
The core language does **not** mandate a scheduler or a particular channel implementation; targets may omit channels entirely.

#### Channel type constructor (syntax vs implementation)
- The **type syntax** `|T|` is part of the language grammar (§5.0).
- The **representation + operations** for channels live in the sysroot (e.g. `platform.channel.*`).
- If a target sysroot omits `platform.channel`, any program that uses `|T|`, `<|`, or `|>` should fail at link time (or earlier if the compiler performs sysroot capability checks).

#### Typing (normative for v1)
- `|>` (send): `( |T| T -- )`
  - consumes the payload value
  - if the payload is `iso T`, the move-only value is transferred to the receiver
- `<|` (recv): `( |T| -- T )`

#### Move-only (`iso`) enforcement is compiler-owned
Even though channels live in the sysroot, **move-only rules are enforced by the compiler**:
- `dup` / `drop` on `iso _` are illegal
- sending an `iso T` via `|>` consumes the unique reference (no implicit copy)

#### Lowering strategy (default)
- `ch msg |>` → resolve and call `platform.channel.send` (or the target’s canonical alias)
- `ch <|`     → resolve and call `platform.channel.recv` (or alias)

The sysroot defines whether channels are lock-free, blocking, polled, etc. (Runtime Contract).



### Sysroot-defined concurrency primitives (tasks + channels)

The language core does not define a scheduler or a task runtime.
Concurrency primitives are provided by the **sysroot**, and the compiler only relies on:

- **Effect annotations** (notably `{suspend}`) to enforce borrow-liveness restrictions.
- **Move-only enforcement** for `iso _` at the typechecker level.

Concrete APIs such as `platform.task.spawn`, `platform.task.yield`, `platform.task.sleep_*`,
and channel operations `platform.channel.send/recv` are sysroot-defined and may be absent on a given target.


## 8) Place model & borrowing checks

### 8.1 Places

Places are addressable storage locations:
- locals (`=> x`)
- fields of places (`p.x`)
- array elements of places (`arr'0`, `arr'(i)`) when `arr : T'N` is a place
- MMIO regs + reg-array elements (`gpio.OUT_SET`, `TIMER.CH'2.CTRL`)
- resources, but only within their `lock` scope

**Indexing creates sub-places.**
If `arr` is a place of type `T'N`, then `arr'idx` is a place of type `T`.
Its address is computed as:

```
addr(arr'idx) = addr(arr) + idx * sizeof(T)
```

Bounds policy:
- if `idx` is a compile-time constant, the compiler should statically reject out-of-range indices
- if `idx` is dynamic, behavior is target/policy defined (trap in debug is recommended)



### 8.2 Borrow operators

- `&place`  → `^T`   (read-only pointer)
- `&!place` → `^!T`  (mutable pointer; only if place is writable)

`&` / `&!` apply only to places (not arbitrary expressions).

### 8.3 Scoped borrows: `&[ ... ]` / `&![ ... ]`

These forms borrow the top stack value and typecheck a block with a temporary borrowed capability.
All scoped borrows are **non-escaping** (§8.4) and are subject to the **suspension liveness rule** (§8.5).

Case A — Arrays (slice view):
- input must be `T'N`
- compiler spills the array to a stable temp
- compiler pushes a `Slice(T)` / `SliceMut(T)` for the block (base pointer + length `N`)
- the slice must be consumed by block exit
- the slice cannot escape the block

Case B — Regions:
- input must be `Region`
- compiler pushes a borrowed region handle for the block (read-only for `&[`, mutable for `&![`)
- the borrowed region handle must be consumed by block exit
- it cannot escape the block

Case C — Any sized value `T` (generalized spill-borrow):
- input must be a **sized** type `T` (not a slice/trait object/unsized future extension)
- compiler spills the value to a stable temp for the duration of the block
- compiler pushes a pointer to that temp:
  - `&[ ... ]` pushes `^T`
  - `&![ ... ]` pushes `^!T`
- the pointer must be consumed by block exit
- it cannot escape the block

This general case enables “borrow destructuring” (§5.6) for structs that originate as stack values.



### 8.4 Non-escape enforcement

“Escape” includes:
- returning the borrowed value
- storing it in globals/resources
- inserting into any `owned` container / heap-like structure
- capturing it in an escaping quotation
- passing it to an unknown/opaque callee without a non-escape guarantee

### 8.5 Suspension (borrow liveness rule)

At any suspension point (any instruction/word with effect `{suspend}`):
- the data stack must contain **no scoped-borrow values**
- no locals may be bound to scoped-borrow values

Policy (v1):
- `lock` quotations must be `{}` (so suspension is already forbidden there).
- suspension inside `&![ ... ]` is rejected.
- suspension inside `&[ ... ]` is allowed only if the borrowed value is not live at that suspension point.

---
## 9) `register-map` lowering

`register-map` expands to a set of **places** whose reads/writes lower to MMIO operations with
access checks and volatile semantics.

### 9.1 Addressability rules
- register places are addressable (`&gpio.REG`)
- field places are not (`&gpio.REG.FIELD` is illegal; fields lower to masked ops)
- reg-array element places are addressable (`&gpio.PINCFG'13`)

### 9.2 Reg-array indexing (`'idx`) (normative)
If a register-map declares an arrayed register:

```forth
0x200 PINCFG u32'40 rw volatile
```

Then `gpio.PINCFG'idx` is a **place** of type `u32` with address:

```
addr(gpio.PINCFG'idx) = base(gpio) + 0x200 + idx * sizeof(u32)
```

Static checks:
- if `idx` is a constant, require `0 <= idx < 40`
- if `idx` is dynamic, bounds behavior is policy-defined (trap recommended for hosted/debug)

### 9.3 Nested maps and stride
For nested maps like:

```forth
0x100 CH TIMER_CH'4 volatile stride=0x20
```

Address of a nested element place is:

```
addr(tim.CH'idx) = base(tim) + 0x100 + idx * stride
```

and then field/register offsets inside `TIMER_CH` apply relative to that base.

### 9.4 Access mode checks
- `ro` registers: stores are rejected
- `wo` registers: loads are rejected
- `rw` registers: both allowed
- volatile attribute forces MMIO loads/stores to remain ordered and not be elided

### 9.5 Field read/write lowering (masked ops)
Given a field `F` in register `R` with `(shift, mask)`:
- read: `tmp = mmio_load32(addr(R));  (tmp >> shift) & mask`
- write: `tmp = mmio_load32(addr(R));  tmp2 = (tmp & ~mask_at_shift) | ((val & mask) << shift);  mmio_store32(addr(R), tmp2)`
The exact sequence depends on field access mode and whether the map provides SET/CLEAR shadow regs.



---

## 10) Mid-end IR (shared)

v1 uses a typed stack IR + basic blocks.

IR ops include:
- consts, arith, cmp
- local bind/get
- calls
- intrinsic control flow
- casts
- borrows (scoped + place borrows)
- address computation for places
- mmio load/store and field ops
- trap

### 10.1 Address computation primitives (needed for places)

The mid-end should have explicit ops for turning a **place** into an address and for computing offsets.

Recommended minimal set:

- `addr_of <place>` → `ptr`
  - yields a typed pointer (`^T` / `^!T`) to the storage backing the place
- `ptr_add ptr bytes` → `ptr`
  - returns `ptr + bytes` (byte offset)
- `ptr_index ptr elem_size idx` → `ptr`
  - shorthand for `ptr_add(ptr, idx * elem_size)` (safe to lower to mul+add)

### 10.2 Lowering `place'idx` (normative)

When the front-end builds a sub-place for array indexing (`arr'idx`) where `arr : T'N`:

1. Compute base pointer:
   - `p0 = addr_of(arr)`   # type: `^T` or `^!T` depending on arr mutability
2. Compute element pointer:
   - `p1 = ptr_index(p0, sizeof(T), idx)`
3. Treat `arr'idx` as the place whose address is `p1`.

So a load becomes:
- `val = load_T(p1)`  (or `mmio_load_T(p1)` if the place is MMIO/volatile)

and a store becomes:
- `store_T(p1, val)`  (or `mmio_store_T(p1, val)`)

Bounds enforcement:
- if `idx` is constant, reject out of range in the front-end
- if `idx` is dynamic and checks are enabled, emit a compare + trap before computing `p1`

### 10.3 Lowering borrow destructuring (field pointer computation)

For `=> { &x &y }` when top-of-stack is `base : ^S` / `^!S`:

For each field `fi` with byte offset `off(fi)` and type `Ti`:
- `p_fi = ptr_add(base, off(fi))`
- bind local `fi` to `p_fi` (typed as `^Ti` or `^!Ti`)

No loads occur as part of destructuring; loads happen only when the user uses `@` / `@T`.



---

## 11) Native AOT backend (overview)

v1 strategy:
- compile each word to a real function symbol
- manage a data-stack pointer (dedicated register or implicit global/TLS-like)
- calls are normal ABI calls

Attributes:
- `@Section`, `@Weak`, `@Inline`
- `@ISR(...)` selects target-specific ISR ABI wrapper

Assembler:
- may emit GAS-compatible asm for portability
- optionally FASM for host targets
Spec requirement: valid target objects.

---

## 12) Testing requirements

- golden parse tests
- typecheck error snapshots
- register-map legality tests
- differential tests: native vs bytecode on Linux
- fuzz parser/typechecker

---

## 13) Deferred
- bytecode instruction set + VM runtime spec
- full debug info (DWARF vs custom)
- closures/captures (v1 forbids implicit capture)
