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
  - scoped borrows: `&[ ... ]` / `&![ ... ]` (Arrays → Slices; Regions → borrowed Region handles)
  - channel pipes: `<|` / `|>` (syntax sugar; lower to stdlib/sysroot channel ops)

### 4.3 Name resolution
Resolve:
- word calls
- type names
- struct field accessors (`.field`, `->field`)
- enum variants (`State.Idle`)
- register-map paths (`gpio.CTRL.MODE`, `gpio.PINCFG[13]`)

---

## 5) Type checking and static semantics

### 5.1 Typed stack checking
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

### 5.2 Subtypes (checks inserted by default)
Default insertion points (v1):
- on `as? Subtype` conversions (required)
- on `as Subtype` conversions (policy: either trap-on-fail, or lower to `as?` + trap)
- on word boundaries for parameters/returns that are subtypes (required by default)

Compile-time constants must be validated at compile time.

### 5.3 Contracts (checks inserted by default)
When enabled:
- evaluate `requires` predicates at entry and `ensures` predicates at exit
- if false: trap

Under `@ISR`, contract predicates must be ISR-safe:
- no allocation
- no blocking
- only call ISR-safe words (flag-propagated)

### 5.4 Casting
- `as T` conversion
- `as? T` checked conversion → pushes `ok:bool`
- `bitcast T` same-size reinterpret

Pointer casts are compile-time errors unless `--allow-raw-casts`.

---

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

`<|` and `|>` are parsed as dedicated operator terms and lowered to stdlib/sysroot channel operations.

Typing (recommended v1):
- `|>` (send): `( Chan(T) T -- )` (consumes payload; if payload is `iso T` it is moved)
- `<|` (recv): `( Chan(T) -- T )`

Lowering strategy:
- `ch msg |>` → call `platform.channel.send` or `chan-send`
- `ch <|`     → call `platform.channel.recv` or `chan-recv`

Whether channels are lock-free, blocking, or polled is platform-defined (see Runtime Contract).


## 8) Place model & borrowing checks

### 8.1 Places

Places are addressable storage locations:
- locals (`=> x`)
- fields of places (`p.x`)
- MMIO regs + reg-array elements (`gpio.OUT_SET`, `TIMER.CH[2].CTRL`)
- resources, but only within their `lock` scope

### 8.2 Borrow operators

- `&place`  → `^T`   (read-only pointer)
- `&!place` → `^!T`  (mutable pointer; only if place is writable)

`&` / `&!` apply only to places (not arbitrary expressions).

### 8.3 Scoped borrows: `&[ ... ]` / `&![ ... ]`

These forms borrow the top stack value and typecheck a block with a temporary borrowed capability.

Case A — Arrays:
- input must be `Array(T,N)`
- compiler spills the array to a stable temp, pushes a `Slice(T)` or `SliceMut(T)` for the block
- the slice must be consumed by block exit
- the slice cannot escape the block (return/store/capture/etc.)

Case B — Regions:
- input must be `Region`
- compiler pushes a borrowed region handle for the block (read-only for `&[`, mutable for `&![`)
- the borrowed region handle must be consumed by block exit
- it cannot escape the block

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

- reg places are addressable (`&gpio.REG`)
- field places are not (`&gpio.REG.FIELD` is illegal)
- access modes enforced at compile time
- volatile semantics preserved

---

## 10) Mid-end IR (shared)

Use a typed stack IR + basic blocks. IR ops include:
- consts, arith, cmp
- local bind/get
- calls
- intrinsic control flow
- casts
- borrows
- mmio load/store and field ops
- trap

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
