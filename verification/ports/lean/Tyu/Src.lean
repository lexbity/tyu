import Tyu.Step

/-! The pure-fragment **source-level** semantics (PLAN-VERIFY-3 P9.1, §Q2/§Q8).

Tyu source words are Forth-style expressions over a typed stack. Most of the
frontend's semantic content — borrows, the `&!` ledger, effect contexts,
`lock` lowering, quotations, the scheduler — is carried *structurally* by the
checker and is NOT formalized this cycle (§Q8). What IS formalized is the
**pure fragment**, and this file is that embedding: a source-level step
semantics for the fragment's operators, written directly (values over the
typed data stack, memory through the `ConcreteMem` oracles, MMIO reads as the
injected nondeterminism — §Q13), plus the **transcription** into the IR
concrete semantics of `Tyu.Step` and the transcription theorem (T-S).

## The fragment boundary (the op-locality detector, §Q2/§Q8)

The typing rule is *op-locality*: a source construct is in the fragment iff
its lowering is a local, per-op IR sequence with no hidden intermediate
state. Under that rule the fragment contains exactly:

  - **values** — `constInt`, `constBool`;
  - **typed-stack ops** — `dup`, `drop`, `swap`;
  - **arithmetic / comparison / boolean logic** — `add` `sub` `mul`,
    `cmpLT … cmpNE`, `andB` `orB` `notB`;
  - **memory** — `load`/`store` and `volLoad`/`volStore` (MMIO through the
    aperture oracle);
  - **the single-slot local cell ops** the compiler emits for return-value
    plumbing — `localGet`/`localSet` (in fragment words these are the hidden
    return-slot saves, net-zero and value-restoring on the data stack);
  - **control flow** — `br` `brIf` `ret`.

EXCLUDED (each non-op-local, and why):

  - `cast`/`bitcast` — a *checked subtype cast* lowers to a trap-guarded
    check sequence (`local_get … ; const … ; cmp_* ; trap_if_false`), so the
    source construct and the IR fragment do not agree step-for-step on the
    value level; the IR `cast` op alone is identity but the emitted checks
    are the semantics. Source-surface statements for words with casts are
    refused (`cast-nonlocal`).
  - `trap_if_false` — always emitted as part of a check sequence; a trap
    stop is a *runtime* behavior, not an op-local source construct.
  - `call` — callee expansion is not a per-op transcription (calls
    `calls-unmodeled`); named-predicate contract words therefore stay IR-only.
  - address/pointer materialization (`addr_of`, `addr_of_mut`, `mmio_place`,
    `ptr_add_*`), quotations (`const_str`), effects (`interrupt_*`,
    `scoped_enter`, `task_spawn`).

This is the *shrunken* boundary the plan's §Q2 escape valve describes: what
here is not op-local, the source surface simply does not claim (the
obligation stays IR-surface or open — never silently re-expressed).

## T-S (the transcription theorem)

`Tyu.Src.transcription` is the registry statement: for a fragment word, the
source-level semantics and the IR-level semantics of its transcription are
*the same relation on concrete states and memories*, parameterized over
`(TargetSpec, MemModel-instance)` (§Q3 — the memory oracles and the target
identity are arguments, never constants):

    transcription : outInRange srcW ↭ irOutInRange (transcribe srcW)

The per-op simulation lemmas [`transcription_op`] (each fragment op's step
equals the concrete step of its transcription), composed over blocks
([`transcription_block`]) and over word execution ([`transcription_run`]),
are the machinery; `transcription` is the statement an auditor reads. A
developer's source-surface certificate (a theorem of `src_stmt_*` in a
generated `Gen/` file, harvested with `surface: "source"` and
`relies: ["T-S"]`) discharges an IR-level obligation *through* this theorem.
-/

namespace Tyu.Src

open Tyu.Step

/-! ### The fragment operator set -/

/-- The pure-fragment source operator (see the header for the boundary
rule). Payloads are carried directly (the source form), not through the IR
op's option payloads. -/
inductive Op where
  | constInt (v : Int)
  | constBool (b : Bool)
  | dup | drop | swap
  | add | sub | mul
  | cmpLT | cmpLE | cmpGT | cmpGE | cmpEQ | cmpNE
  | andB | orB | notB
  | load | store
  | volLoad | volStore
  | localGet (n : Nat) | localSet (n : Nat)
  | br (t : Nat) | brIf (t e : Nat) | ret
  deriving Repr, Inhabited, BEq

namespace Op

/-- The canonical `--emit=ir` mnemonic of a fragment op (the subset of
`Tyu.IR.OpForm.mnemonic` the fragment covers). -/
def mnemonic : Op → String
  | .constInt _ => "const_i64"
  | .constBool _ => "const_bool"
  | .dup => "dup"
  | .drop => "drop"
  | .swap => "swap"
  | .add => "add_i64"
  | .sub => "sub_i64"
  | .mul => "mul_i64"
  | .cmpLT => "cmp_lt"
  | .cmpLE => "cmp_le"
  | .cmpGT => "cmp_gt"
  | .cmpGE => "cmp_ge"
  | .cmpEQ => "cmp_eq"
  | .cmpNE => "cmp_ne"
  | .andB => "and_bool"
  | .orB => "or_bool"
  | .notB => "not_bool"
  | .load => "load"
  | .store => "store"
  | .volLoad => "vol_load"
  | .volStore => "vol_store"
  | .localGet _ => "local_get"
  | .localSet _ => "local_set"
  | .br _ => "br"
  | .brIf _ _ => "br_if"
  | .ret => "ret"

end Op

/-- The op-local transcription: each fragment op maps to the concrete IR op
with the identical payload (one op per source construct — the op-locality
rule). -/
def transcribeOp : Op → ConcreteOp
  | .constInt v => (ConcreteOp.opMk .const_i64).setConst v
  | .constBool b => (ConcreteOp.opMk .const_bool).setConstBool b
  | .dup => ConcreteOp.opMk .dup
  | .drop => ConcreteOp.opMk .drop
  | .swap => ConcreteOp.opMk .swap
  | .add => ConcreteOp.opMk .add_i64
  | .sub => ConcreteOp.opMk .sub_i64
  | .mul => ConcreteOp.opMk .mul_i64
  | .cmpLT => ConcreteOp.opMk .cmp_lt
  | .cmpLE => ConcreteOp.opMk .cmp_le
  | .cmpGT => ConcreteOp.opMk .cmp_gt
  | .cmpGE => ConcreteOp.opMk .cmp_ge
  | .cmpEQ => ConcreteOp.opMk .cmp_eq
  | .cmpNE => ConcreteOp.opMk .cmp_ne
  | .andB => ConcreteOp.opMk .and_bool
  | .orB => ConcreteOp.opMk .or_bool
  | .notB => ConcreteOp.opMk .not_bool
  | .load => ConcreteOp.opMk .load
  | .store => ConcreteOp.opMk .store
  | .volLoad => ConcreteOp.opMk .vol_load
  | .volStore => ConcreteOp.opMk .vol_store
  | .localGet n => (ConcreteOp.opMk .local_get).setSlot n
  | .localSet n => (ConcreteOp.opMk .local_set).setSlot n
  | .br t => (ConcreteOp.opMk .br).setBrTgt t
  | .brIf t e => (ConcreteOp.opMk .br_if).setBrIf (t, e)
  | .ret => ConcreteOp.opMk .ret

/-! ### The source-level step -/

/-- The comparison order of a fragment comparison op (the row-level
`cmp_*` semantics; `False` for the non-comparison forms, which the defined
cases never reach). -/
def cmpKind : Op → Value → Value → Bool
  | .cmpLT, a, b => a < b
  | .cmpLE, a, b => a ≤ b
  | .cmpGT, a, b => a > b
  | .cmpGE, a, b => a ≥ b
  | .cmpEQ, a, b => a == b
  | .cmpNE, a, b => a ≠ b
  | _, _, _ => false

/-- The source-level step: written directly over the typed stack and the
memory oracles (the fragment ops never trap — `Outcome.trap` is produced
only by `trap_if_false`, which the fragment excludes, so the `.trap` branch
is unreachable-by-construction for fragment ops). Control ops are *not*
stepped here (they are intercepted by `runBlock`, exactly as the IR
semantics does); a total definition is provided for the per-op law. -/
def stepOp (_spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (o : Op) (st : State) : ConcreteMem × Outcome :=
  match o with
  | .constInt v => (mem, .ok (State.push1 st v))
  | .constBool b => (mem, .ok (State.push1 st (if b then 1 else 0)))
  | .dup =>
      let (s1, v) := State.pop1 st
      (mem, .ok (State.pushMany s1 [v, v]))
  | .drop =>
      let (s1, _) := State.pop1 st
      (mem, .ok s1)
  | .swap =>
      let (s1, b) := State.pop1 st
      let (s2, a) := State.pop1 s1
      (mem, .ok (State.pushMany s2 [a, b]))
  | .add =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (wrapI64 (a + b))))
  | .sub =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (wrapI64 (a - b))))
  | .mul =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (wrapI64 (a * b))))
  | .cmpLT | .cmpLE | .cmpGT | .cmpGE | .cmpEQ | .cmpNE =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (if cmpKind o a b then 1 else 0)))
  | .andB =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (if a ≠ 0 && b ≠ 0 then 1 else 0)))
  | .orB =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (if a ≠ 0 || b ≠ 0 then 1 else 0)))
  | .notB =>
      let (s1, v) := State.pop1 st
      (mem, .ok (State.push1 s1 (if v == 0 then 1 else 0)))
  | .load =>
      let (s1, addr) := State.pop1 st
      (mem, .ok (State.push1 s1 (mem.load addr)))
  | .store =>
      let (s1, v) := State.pop1 st
      let (s2, addr) := State.pop1 s1
      (mem.record addr v, .ok s2)
  | .volLoad =>
      let (s1, _) := State.pop1 st
      (mem, .ok (State.push1 s1 mem.mmioRead))
  | .volStore =>
      let (s1, _) := State.pop1 st
      let (s2, _) := State.pop1 s1
      (mem, .ok s2)
  | .localGet n =>
      (mem, .ok (State.push1 st (st.locals.getD n 0)))
  | .localSet n =>
      let (s1, v) := State.pop1 st
      (mem, .ok (State.setLocal s1 n v))
  | .br _ | .ret => (mem, .ok st)
  | .brIf t e =>
      -- the op-level `br_if` pops its condition (the *block* run intercepts
      -- `brIf` before the step, so this edge case is the op-law only; the
      -- interception uses the same pop in the routed End).
      let (s1, _) := State.pop1 st
      (mem, .ok s1)

/-! ### Blocks and words -/

/-- A source fragment block: id + op list (the terminator is the last op). -/
structure Block where
  id : Nat
  ops : List Op
  deriving Repr, Inhabited

namespace Block

def empty : Block := { id := 0, ops := [] }

/-- The block-run routing (mirrors `Tyu.Step.Block.runBlock`): control ops
are intercepted (the terminator), every other op steps via `Src.stepOp`. -/
def runBlock (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (ops : List Op) (st : State) : ConcreteMem × Tyu.Step.Block.End :=
  match ops with
  | [] => (mem, .ret st)
  | o :: rest =>
      match o with
      | .ret => (mem, .ret st)
      | .br t => (mem, .go t st)
      | .brIf t e =>
          let (s1, v) := State.pop1 st
          (mem, .brIf t e v s1)
      | _ =>
          match stepOp spec mem o st with
          | (m1, .trap) => (m1, .trap)
          | (m1, .ok s1) => runBlock spec m1 rest s1

end Block

/-- A source fragment word: a block list (indexed by block id) + the entry
block. -/
structure Word where
  blocks : List Block
  entry : Nat
  deriving Repr, Inhabited

namespace Word

/-- The word-level run: fuel steps through the CFG from block `entry`
(mirrors `Tyu.Step.runWord`). `fuel = 0` means the run did not terminate
within the budget. -/
def run (blocks : List Block) (spec : Tyu.IR.TargetSpec) (entry : Nat) (fuel : Nat)
    (mem : ConcreteMem) (st : State) : ConcreteMem × Option State :=
  match fuel with
  | 0 => (mem, none)
  | fuel' + 1 =>
      let b := blocks.getD entry Block.empty
      match Block.runBlock spec mem b.ops st with
      | (mem1, .ret st1) => (mem1, some st1)
      | (mem1, .go t st1) => Word.run blocks spec t fuel' mem1 st1
      | (mem1, .brIf t e cond st1) =>
          if cond ≠ 0 then Word.run blocks spec t fuel' mem1 st1
          else Word.run blocks spec e fuel' mem1 st1
      | (mem1, .trap) => (mem1, none)

/-- Termination: a word run that completes within the fuel budget. -/
def Terminates (blocks : List Block) (spec : Tyu.IR.TargetSpec) (entry : Nat) (fuel : Nat)
    (mem : ConcreteMem) (st : State) : Prop :=
  ∃ st', Word.run blocks spec entry fuel mem st = (mem, some st')

end Word

/-! ### Statement-side forms over the source semantics -/

/-- `lo ≤ v ≤ hi` on a concrete value. -/
def inRange (v : Int) (lo hi : Int) : Prop := lo ≤ v ∧ v ≤ hi

/-- The i-th input value at the entry stack (from the bottom, index 0 — the
canonical `in.i`). -/
def inputAt (σ : State) (i : Nat) : Int :=
  σ.stack.getD i 0

/-- The i-th output value at the exit stack (from the top; `out.0` is the
top of the return stack). Total: an index past the stack top reads 0;
`stack.length - 1 - i` saturates at 0 so the projection never underflows.
(Same reading as `Tyu.Gen.Stmt.outputAt`; the correspondence is
review-recorded T-F2.) -/
def outputAt (σ : State) (i : Nat) : Int :=
  σ.stack.getD (σ.stack.length - 1 - i) 0

/-- The source-side `subtype-range` claim for a word output: every
terminating source run's i-th output value lies in `[lo, hi]`. -/
def outInRange (blocks : List Block) (entry : Nat) (i : Nat) (lo hi : Int) : Prop :=
  ∀ (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (fuel : Nat) (σ₀ σf : State),
    Word.run blocks spec entry fuel mem σ₀ = (mem, some σf) →
    inRange (outputAt σf i) lo hi

/-- The source-side `subtype-range` claim for a word input (a sound
over-claim — the callee cannot know its callers; in practice unprovable). -/
def inInputRange (i : Nat) (lo hi : Int) : Prop :=
  ∀ (_spec : Tyu.IR.TargetSpec) (_mem : ConcreteMem) (σ₀ : State),
    inRange (inputAt σ₀ i) lo hi

/-- The source-side `mmio-bounds` claim (`OffsetLE` with a compile-time
offset): the emulated-aperture access at `off` of access-width `width` fits
the aperture of size `size` on every terminating source run. -/
def offsetWithin (blocks : List Block) (entry : Nat) (off width size : Nat) : Prop :=
  ∀ (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (fuel : Nat) (σ₀ : State),
    (∃ σf, Word.run blocks spec entry fuel mem σ₀ = (mem, some σf)) → off + width ≤ size

/-- Evaluate a contract-predicate fragment word over a concrete argument
stack: the predicate runs terminate and their top-of-stack is nonzero. -/
def predHolds (pred : List Block) (spec : Tyu.IR.TargetSpec) (fuel : Nat)
    (mem : ConcreteMem) (args : List Int) : Prop :=
  ∃ σf, Word.run pred spec 0 fuel mem { stack := args, locals := [] } = (mem, some σf) ∧
    σf.stack.getLastD 0 ≠ 0

/-- The source-side `contract-post` claim: the word's `ensures` predicate
holds over the direct `out.i` arguments at every terminating source run. -/
def predHoldsOnOuts (w pred : List Block) (wEntry : Nat) (_i : Nat) (_lo _hi : Int)
    (args : List Int) : Prop :=
  ∀ (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (fuel : Nat) (σ₀ σf : State),
    Word.run w spec wEntry fuel mem σ₀ = (mem, some σf) →
    predHolds pred spec fuel mem args

/-! ### Transcription -/

/-- Transcribe a block (op-list pointwise + id). -/
def transcribeBlock (b : Block) : Tyu.Step.Block :=
  { id := b.id, ops := b.ops.map transcribeOp }

/-- The transcription of a block is the identity on the default (missing)
block. -/
theorem transcribeBlock_empty : transcribeBlock Block.empty = Tyu.Step.Block.empty := by
  rfl

/-- The `.ops` of a transcribed block is the pointwise transcription of its
op list (the projection form the run theorem's block lookup reduces to). -/
theorem transcribeBlock_ops (b : Block) : (transcribeBlock b).ops = b.ops.map transcribeOp := by
  rfl

/-- `List.map` and `getD` commute: the entry block of the transcribed word
is the transcription of the entry block of the source word. -/
theorem map_getD_block (ws : List Block) (e : Nat) :
    (ws.map transcribeBlock).getD e Tyu.Step.Block.empty = transcribeBlock (ws.getD e Block.empty) := by
  induction ws generalizing e with
  | nil => simp [transcribeBlock, Block.empty, Tyu.Step.Block.empty]
  | cons b rest ih =>
      cases e with
      | zero => simp [transcribeBlock]
      | succ e' =>
          calc
            (List.map transcribeBlock (b :: rest)).getD (e' + 1) Tyu.Step.Block.empty
                = (List.map transcribeBlock rest).getD e' Tyu.Step.Block.empty := by
                  simp
            _ = transcribeBlock (rest.getD e' Block.empty) := ih e'
            _ = transcribeBlock ((b :: rest).getD (e' + 1) Block.empty) := by
                  simp

-- ---------------------------------------------------------------------------
-- Per-op simulation
-- ---------------------------------------------------------------------------

/-- The per-op simulation lemma (T-S's first rung): a fragment op's source
step equals the concrete step of its transcription, for every
`(TargetSpec, MemModel-instance)` — the op-locality claim, case by case. -/
theorem transcription_op (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (o : Op) (st : State) :
    stepOp spec mem o st = Tyu.Step.stepOp spec mem (transcribeOp o) st := by
  cases o <;> simp [stepOp, Tyu.Step.stepOp, Tyu.Step.stepOp.cmpKind, transcribeOp, cmpKind,
                    Tyu.Step.ConcreteOp.opMk, Tyu.Step.ConcreteOp.setConst,
                    Tyu.Step.ConcreteOp.setConstBool, Tyu.Step.ConcreteOp.setSlot,
                    Tyu.Step.ConcreteOp.setBrTgt, Tyu.Step.ConcreteOp.setBrIf,
                    State.push1, State.pushMany, State.pop1, State.pop2, State.setLocal]

-- ---------------------------------------------------------------------------
-- Block-level composition
-- ---------------------------------------------------------------------------

/-- The block-level simulation: the source block-run equals the IR block-run
of the transcribed ops — the per-op lemmas composed over a block body
(structural induction on the op list, one case per fragment op). -/
theorem transcription_block (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem)
    (ops : List Op) (st : State) :
    Block.runBlock spec mem ops st = Tyu.Step.Block.runBlock spec mem (ops.map transcribeOp) st := by
  induction ops generalizing mem st with
  | nil => rfl
  | cons o rest ih =>
      cases o with
      | ret => rfl
      | br t => rfl
      | brIf t e =>
          unfold Block.runBlock Tyu.Step.Block.runBlock
          rw [transcription_op spec mem (.brIf t e) st]
          cases hp : State.pop1 st with
          | mk s1 v =>
              simp [transcribeOp, Tyu.Step.ConcreteOp.opMk, Tyu.Step.ConcreteOp.setBrIf]
      | constInt v =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem (.constInt v) st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp (.constInt v)) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | constBool b =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem (.constBool b) st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp (.constBool b)) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | dup =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .dup st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .dup) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | drop =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .drop st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .drop) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | swap =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .swap st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .swap) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | add =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .add st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .add) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | sub =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .sub st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .sub) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | mul =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .mul st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .mul) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | cmpLT =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .cmpLT st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .cmpLT) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | cmpLE =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .cmpLE st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .cmpLE) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | cmpGT =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .cmpGT st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .cmpGT) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | cmpGE =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .cmpGE st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .cmpGE) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | cmpEQ =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .cmpEQ st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .cmpEQ) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | cmpNE =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .cmpNE st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .cmpNE) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | andB =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .andB st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .andB) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | orB =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .orB st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .orB) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | notB =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .notB st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .notB) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | load =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .load st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .load) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | store =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .store st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .store) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | volLoad =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .volLoad st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .volLoad) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | volStore =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem .volStore st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp .volStore) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | localGet n =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem (.localGet n) st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp (.localGet n)) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1
      | localSet n =>
        unfold Block.runBlock Tyu.Step.Block.runBlock
        rw [transcription_op spec mem (.localSet n) st]
        cases h : Tyu.Step.stepOp spec mem (transcribeOp (.localSet n)) st with
        | mk m1 out =>
            cases out with
            | trap => simp [h]; rfl
            | ok s1 => simp [h]; exact ih m1 s1


theorem transcription_run (blocks : List Block) (spec : Tyu.IR.TargetSpec) (entry fuel : Nat)
    (mem : ConcreteMem) (st : State) :
    Word.run blocks spec entry fuel mem st =
      Tyu.Step.runWord (blocks.map transcribeBlock) spec entry fuel mem st := by
  induction fuel generalizing entry mem st with
  | zero => rfl
  | succ fuel' ih =>
      rw [Word.run, Tyu.Step.runWord]
      rw [map_getD_block]
      have hb := transcription_block spec mem (blocks.getD entry Block.empty).ops st
      cases h1 : Block.runBlock spec mem (blocks.getD entry Block.empty).ops st with
      | mk mem1 end1 =>
          cases end1 with
          | ret σ1 =>
              rw [transcribeBlock_ops, ← hb, h1]
          | go t σ1 =>
              rw [transcribeBlock_ops, ← hb, h1]
              exact ih t mem1 σ1
          | brIf t e cond σ1 =>
              rw [transcribeBlock_ops, ← hb, h1]
              by_cases hc : cond ≠ 0
              · simp [hc]; exact ih t mem1 σ1
              · simp [hc]; exact ih e mem1 σ1
          | trap =>
              rw [transcribeBlock_ops, ← hb, h1]

theorem transcription_run_iff (blocks : List Block) (spec : Tyu.IR.TargetSpec)
    (entry fuel : Nat) (mem : ConcreteMem) (σ σf : State) :
    Word.run blocks spec entry fuel mem σ = (mem, some σf) ↔
      Tyu.Step.runWord (blocks.map transcribeBlock) spec entry fuel mem σ = (mem, some σf) := by
  constructor <;> intro h <;> simpa [transcription_run blocks] using h

/-! ### The IR-side claims over the transcription -/

/-- The IR-side out-range claim over the transcribed word (the claim a
transcribed statement — and, by the T-F2 correspondence, the rendered
`Gen/Stmt` statement — makes about the word's outputs). -/
def irOutInRange (blocks : List Block) (entry : Nat) (i : Nat) (lo hi : Int) : Prop :=
  ∀ (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (fuel : Nat) (σ₀ σf : State),
    Tyu.Step.runWord (blocks.map transcribeBlock) spec entry fuel mem σ₀ = (mem, some σf) →
    inRange (outputAt σf i) lo hi

/-- The IR-side `mmio-bounds` claim over the transcribed word. -/
def irOffsetWithin (blocks : List Block) (entry : Nat) (off width size : Nat) : Prop :=
  ∀ (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (fuel : Nat) (σ₀ : State),
    (∃ σf, Tyu.Step.runWord (blocks.map transcribeBlock) spec entry fuel mem σ₀ = (mem, some σf)) →
      off + width ≤ size

-- ---------------------------------------------------------------------------
-- T-S: the registry statement
-- ---------------------------------------------------------------------------

/-- T-S, out-range form: a source-level out-range claim is equivalent to the
transcribed IR out-range claim — a source-level certificate discharges the
IR obligation *in composition with this theorem* (§Q2). -/
theorem transcription (blocks : List Block) (entry : Nat) (i : Nat) (lo hi : Int) :
    outInRange blocks entry i lo hi ↔ irOutInRange blocks entry i lo hi := by
  unfold outInRange irOutInRange
  apply Iff.intro
  · exact fun hOut spec mem fuel s0 sf hr =>
      hOut spec mem fuel s0 sf (by simpa [transcription_run blocks] using hr)
  · exact fun hI spec mem fuel s0 sf hs =>
      hI spec mem fuel s0 sf (by simpa [transcription_run blocks] using hs)

/-- T-S, offset form: the source and transcribed IR `mmio-bounds` claims
coincide. -/
theorem transcription_offset (blocks : List Block) (entry : Nat) (off width size : Nat) :
    offsetWithin blocks entry off width size ↔ irOffsetWithin blocks entry off width size := by
  unfold offsetWithin irOffsetWithin
  apply Iff.intro
  · exact fun hO spec mem fuel s0 htr =>
      match htr with
      | ⟨sf, hr⟩ => hO spec mem fuel s0 ⟨sf, by simpa [transcription_run blocks] using hr⟩
  · exact fun hI spec mem fuel s0 htr =>
      match htr with
      | ⟨sf, hr⟩ => hI spec mem fuel s0 ⟨sf, by simpa [transcription_run blocks] using hr⟩

/-- The value the exit state of the one-constant word carries on top: the
pushed constant (the statement-side "exit = entry ++ [v]" reading). -/
theorem const_ret_output (v : Int) (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem)
    (σ₀ : State) (fuel' : Nat) :
    outputAt { stack := σ₀.stack ++ [v], locals := σ₀.locals } 0 = v := by
  unfold outputAt
  simp [Nat.sub_zero, List.getLastD]

/-- The exit state of a one-constant word (`const v; ret`): the value `v`
is pushed on the entry stack and the run terminates after the first block.
Total exit-state helper for the source-surface worked example. -/
theorem const_ret_exit (v : Int) (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem)
    (σ₀ : State) (fuel' : Nat) :
    Word.run [ { id := 0, ops := [Op.constInt v, Op.ret] } ] spec 0 (fuel' + 1) mem σ₀ =
      (mem, some { stack := σ₀.stack ++ [v], locals := σ₀.locals }) := by
  simp [Word.run, Block.runBlock, stepOp, State.push1, State.pop1]

/-- `Word.run` with no fuel never terminates (the fuel-budget base case). -/
theorem run_zero_fuel (blocks : List Block) (spec : Tyu.IR.TargetSpec) (entry : Nat)
    (mem : ConcreteMem) (σ : State) :
    Word.run blocks spec entry 0 mem σ = (mem, none) := by
  rfl

end Tyu.Src