# Appendix A — Language Reference

> Status: condensed, verified against the compiler at draft time. Where
> the older `tyu-technical-manual.md` disagrees with this appendix, this
> appendix wins — it was checked against the current tree (the manual
> still shows the retired `requires [ … ]` predicate syntax, for
> example).

## A.1 Table A.1 — the built-in words

The compiler ships exactly sixteen words. Everything else in the
vocabulary is written in Tyu (see chapter 2.8, and the sysroot sources).
`net` is the change in stack depth; `high` the peak (chapter 6).

| Word | Declared effect | net | high | Notes |
|------|-----------------|-----|------|-------|
| `dup`  | `( i64 -- i64 i64 )` | +1 | 1 | copying is a privilege (ch. 7) |
| `drop` | `( i64 -- )`         | −1 | 0 | discarding too (ch. 7) |
| `swap` | `( i64 i64 -- i64 i64 )` | 0 | 0 | exchange the top two |
| `+`    | `( i64 i64 -- i64 )` | −1 | 0 | signed arithmetic |
| `-`    | `( i64 i64 -- i64 )` | −1 | 0 | |
| `*`    | `( i64 i64 -- i64 )` | −1 | 0 | |
| `>` `<` `>=` `<=` `==` | `( i64 i64 -- bool )` | −1 | 0 | comparisons yield `bool` |
| `and` `or` | `( bool bool -- bool )` | −1 | 0 | |
| `not`  | `( bool -- bool )`   | 0  | 0 | |
| `call` | `( quot -- … )`      | —  | ⊤ | needs an annotated quotation; high is ⊤ because the quotation's own bound governs (E5103 on ⊤-forbidding profiles) |
| `platform.task.yield` | `( -- )` performs `suspend` | 0 | 0 | builtin effect carrier |

Everything else — shuffles (`over`, `tuck`), printing, regions, channels,
tasks, all of it — is sysroot source, written in Tyu.

## A.2 Lexical syntax

- Source is UTF-8. Comments run `#` to end of line.
- Integer literals: `42`, `-5`, `0xFF`, `0b1010`. String literals:
  `"text\n"`.
- Every term is either a literal (pushes) or a word (transforms).
- Punctuation with meaning: `:` … `;` (word definition), `( … -- … )`
  (stack effect), `[` `]` (quotation), `{` `}` (data/effect blocks),
  `=>` (bind local), `->` (field pointer), `'` (array suffix/index),
  `|T|` (channel type), `<|` `|>` (channel receive/send), `&` `&!`
  (borrows), `&[` `&![` (scoped borrows), `..` (range), `@T`/`!T`
  (typed load/store through a pointer).

## A.3 Types

Primitives: `u8 u16 u32 u64 usize`, `i8 i16 i32 i64 isize`, `bool`.
Pointers: `^T` shared-read, `^!T` mutable. Composites: `struct`,
`enum : <repr>`, `subtype Base = Base range lo .. hi`, `T'N` fixed
arrays. Type grammar (qualifiers outermost):

```text
Type     ::= Qual* TypePref          Qual ::= "iso" | "owned" | "resource" | "lock"
TypePref ::= PtrPref* TypePost       PtrPref ::= "^" | "^!"
TypePost ::= TypePrim ArraySuf*      TypePrim ::= Ident | "(" Type ")" | ChanType
ChanType ::= "|" Type "|"            ArraySuf ::= "'" ConstInt
```

Conversions: `as T` (checked; traps SUBTYPE_FAIL on range violation),
`as? T` (value + ok flag), `bitcast T` (equal size required, E3304).
Raw pointer casts are compile errors (E3305) on the current tree, with
or without `--allow-raw-casts`.

## A.4 Declarations

```tyu
module Name;
import Path { word1 word2 };        # or: import Path;
export { word1 };

struct Point  x : i32  y : i32  end;
enum State : u8  Idle = 0x00  Run = 0x01  end;      # explicit numbers required
subtype Percent = i64 range 0 .. 100;
resource Counter : i64;
register-map GPIO … end;
const name = <typed literal>;       # parsed on the current tree; not yet
                                    # readable in expressions (known gap)
@interrupt(VECTOR) : handler ( -- ) … ;
```

## A.5 Word signatures and clauses

```tyu
: word ( ins -- outs )
  performs {effects}        # what the word does
  requires {capabilities}   # what it needs granted (recorded; enforcement
                            # is via the lexical contexts on this tree)
  needs   [ precondition ]  # checked after the call, before the body
  ensures [ postcondition ] # checked after the body, before the return
  … body … ;
```

Contract predicates are ordinary checked code and must obey the shape
rules: end with exactly one `bool` on top (E3310), the final value is a
`bool` (E3311), declared inputs survive unchanged (E3312).

## A.6 Control

```tyu
condition [ true-branch ] [ false-branch ] if     # expression; branches must agree
[ condition ] [ body ] while    # cond: net 0, ends bool; body: net 0 (E3257/3258)
[ body ] loop                   # net-zero body; performs diverge
R lock [ … ]                    # grants write(R); forbids suspend/interrupt;
                                # stack-neutral; no nesting (E5002)
return                          # only in words with an explicit stack effect
```

`if`-as-expression branches must leave identical stacks (E3246 depth,
E3247 content). Escaping quotations (passed to `spawn`, stored, or
`call`ed) must carry an explicit annotation: `[ ( ins -- outs ) … ]`
(E3760).

## A.7 Locals

`=> name` pops the top value into a word-scoped immutable local. Borrow
destructuring `=> { &x &y }` binds field pointers inside a borrow block.

## A.8 Memory

Regions (platform `mem`): `region-create ( usize -- Region )`,
`region-alloc ( Region usize -- ptr_mut )` performs `alloc`,
`region-reset`, `region-destroy`. Pointers are consumed by typed loads
`@T` and stores `!T`. Borrows: `&place` / `&!place`; scoped blocks
`&[ … ]` / `&![ … ]`; the lease ends at the bracket (E5020); conflicting
live mints are E5021; `dup` of a mutable borrow is E5022.

## A.9 MMIO

```tyu
register-map Name
  0x00 REG u32 rw volatile { field 0..8 u16 rw }
end;
const dev = Name @ board.instance ;
```

The map is checked row-by-row against the platform descriptor (E3644
unknown instance, E3647 row mismatch, E3611 misaligned, E3608 bitfield
not addressable, E3640 no descriptor). Descriptor `write_kind` (`plain`,
`w1s`, `w1c`, `xor`) and `read_kind` (`plain`, `effectful`) select the
lowering. `volatile` accesses are never elided, merged, or reordered.

## A.10 Tasks and channels (platform-provided)

`platform.task.run ( quot -- )` discharges `suspend` (grants
`suspendable`); `spawn ( quot -- Task )` (annotated quotation, E3760);
`join ( Task -- )` performs `suspend`; `yield`, `sleep-ms`, `sleep-us`
perform `suspend`. `platform.channel`: `make ( -- |T| )`,
`send ( |T| T -- )` (a move), `recv ( |T| -- T )`. Hosted ships both;
boards ship what their descriptor declares — importing an absent module
is E2201.

## A.11 Hosted entry ABI and markers

`main` is a word and must return exactly one value — the exit code
(E1018). Clean programs emit the completion marker `S\n` (via the
platform log word) before pushing `0`; failing checks emit `F`.
`tyu run` verdicts: silent + exit 0 (pass), `NO_COMPLETION` (trap/early
death, native exit status = trap code), `FAIL_MARKER` (F seen, exit 2),
`HANG` (timeout, exit 124), `EXIT_MISMATCH` (wrong exit code with
marker).
