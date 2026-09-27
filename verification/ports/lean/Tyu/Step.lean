import Tyu.IR.Op
import Tyu.IR.Semantics
import Tyu.IR.Target

/-! The concrete step semantics (PLAN-VERIFY-3 P4.1).

The port's operational layer: real values over the data stack,
width-parameterized wrapping arithmetic, memory through a parametric
`ConcreteMem` (loads and MMIO reads are nondeterministic oracles — §Q13: the
oracle is a *parameter*, never a fixed value), traps, block/word-level runs
with a fuel-based termination witness.

T-C (stack algebra, `Tyu/Sound.lean`) is proved *against* this semantics:
per-op depth lemmas connect the generated `OpForm.pops/pushes` to the actual
stack-length delta of `stepOp`, and the monoid composition is verified over
terminating runs.

`ConcreteOp` carries the payloads the step consults. For `call` the sig is
carried explicitly (the canonical op text does not); `callSig = none` falls
back to the representative (1,1) sig. -/

namespace Tyu.Step

abbrev Value := Int

/-- The i64 two's-complement wraparound: `x` mod 2^64 interpreted signed.
Data ops wrap at 64 bits on *every* target — the runtime emulates 64-bit
arithmetic on 32-bit targets (`verifier::interval`'s width-relativism
note). -/
def S64 : Int := 18446744073709551616
def S63 : Int := 9223372036854775808

def wrapI64 (x : Int) : Int :=
  let m := x.emod S64
  if m ≥ S63 then m - S64 else m

/-- The concrete stack state: values over the data stack (top = last) and
the locals. -/
structure State where
  stack : List Value
  locals : List Value
  deriving Repr, Inhabited

namespace State

def fresh (localsCap : Nat) : State :=
  { stack := [], locals := List.replicate localsCap 0 }

def push1 (st : State) (v : Value) : State :=
  { st with stack := st.stack ++ [v] }

def pushMany (st : State) (vs : List Value) : State :=
  vs.foldl (fun s v => push1 s v) st

/-- Map with index (locals update). -/
def mapIdx {α : Type} (l : List α) (f : Nat → α → α) : List α :=
  let rec go : Nat → List α → List α
    | _, [] => []
    | i, x :: xs => f i x :: go (i + 1) xs
  go 0 l

def setLocal (st : State) (idx : Nat) (v : Value) : State :=
  if idx < st.locals.length
    then { st with locals := mapIdx st.locals (fun i x => if i == idx then v else x) }
    else st

/-- Pop the top value; an empty stack pops the no-op value 0 (the abstract
engine's `pop_val` default; a well-typed run never pops empty). -/
def pop1 (st : State) : State × Value :=
  match st.stack.reverse with
  | [] => (st, 0)
  | v :: rest => ({ st with stack := rest.reverse }, v)

theorem pop1_length (st : State) :
    (State.pop1 st).1.stack.length = st.stack.length - min 1 st.stack.length := by
  cases h : st.stack.reverse with
  | nil =>
      have hlen : st.stack.length = 0 := by
        rw [← List.length_reverse, h]
        simp
      have hstack : st.stack = [] := List.length_eq_zero_iff.mp hlen
      simp [State.pop1, hstack]
  | cons v rest =>
      have hstack : st.stack = (v :: rest).reverse := by
        rw [← h]
        simp
      have hlen : ((v :: rest).reverse).length = rest.length + 1 := by
        simp [List.reverse_cons]
      have hrest : rest.reverse.length = rest.length := by simp
      simp [State.pop1, hstack]

def pop2 (st : State) : State × Value × Value :=
  let (s1, b) := pop1 st
  let (s2, a) := pop1 s1
  (s2, a, b)

/-- Pop `n` values; the returned list is top-first. The stack saturates at
empty (default values); `popN_length` pins the length math. -/
def popN : State → Nat → State × List Value
  | st, 0 => (st, [])
  | st, k + 1 =>
      match State.pop1 st with
      | (s1, v) => match popN s1 k with
          | (s2, rest) => (s2, v :: rest)

/-- The length math of `popN` when the stack has at least `n` elements (the
well-typed case — T-C's runs never pop empty): `len' = len - n`. -/
theorem popN_length_ge : ∀ (st : State) (n : Nat),
    n ≤ st.stack.length → (popN st n).1.stack.length = st.stack.length - n := by
  intro st n
  induction n generalizing st with
  | zero => simp [popN]
  | succ k ih =>
      intro h
      have hnonempty : 1 ≤ st.stack.length := by omega
      have hpop : (State.pop1 st).1.stack.length = st.stack.length - 1 := by
        have := State.pop1_length st
        rw [Nat.min_eq_left] at this
        · exact this
        · exact hnonempty
      have hk : k ≤ (State.pop1 st).1.stack.length := by omega
      have ih' := ih (State.pop1 st).1 hk
      simp [popN]
      change (popN (State.pop1 st).1 k).1.stack.length = st.stack.length - (k + 1)
      rw [ih', hpop]
      omega

/-- Pop `1` (well-typed): `len' = len - 1`. -/
theorem pop1_length_ge (st : State) (h : 1 ≤ st.stack.length) :
    (State.pop1 st).1.stack.length = st.stack.length - 1 := by
  have := State.pop1_length st
  rw [Nat.min_eq_left] at this
  · exact this
  · exact h

/-- A nonempty stack has length ≥ 1 (the numeric form T-C's length lemmas
need; `max0`-style guards from stepOk simplify to this). -/
theorem length_nonneg_one {l : List Value} (h : l ≠ []) : 1 ≤ l.length := by
  by_cases hc : l.length = 0
  · exact False.elim (h (List.length_eq_zero_iff.mp hc))
  · have hle : 1 ≤ l.length := by omega
    exact hle

/-- Pop `1` on a nonempty stack: `len' = len - 1`. -/
theorem pop1_length_nonempty (st : State) (h : st.stack ≠ []) :
    (State.pop1 st).1.stack.length = st.stack.length - 1 := by
  have hlen : 1 ≤ st.stack.length := by
    by_cases h1 : 1 ≤ st.stack.length
    · exact h1
    · have h0 : st.stack.length = 0 := by omega
      exact False.elim (h (List.length_eq_zero_iff.mp h0))
  exact State.pop1_length_ge st hlen

/-- Pop `2` (well-typed): `len' = len - 2`. -/
theorem pop2_length_ge (st : State) (h : 2 ≤ st.stack.length) :
    (State.pop2 st).1.stack.length = st.stack.length - 2 := by
  have h1 : 1 ≤ st.stack.length := by omega
  have hp1 := State.pop1_length_ge st h1
  have h2 : 1 ≤ (State.pop1 st).1.stack.length := by rw [hp1]; omega
  have hp2 := State.pop1_length_ge (State.pop1 st).1 h2
  simp [State.pop2]
  rw [hp2, hp1]
  omega

/-- Push one value: `len' = len + 1`. -/
@[simp] theorem push1_length (st : State) (v : Value) :
    (State.push1 st v).stack.length = st.stack.length + 1 := by
  simp [State.push1]

/-- Push many: `len' = len + vs.length`. -/
@[simp] theorem pushMany_length (st : State) (vs : List Value) :
    (State.pushMany st vs).stack.length = st.stack.length + vs.length := by
  induction vs generalizing st with
  | nil => simp [State.pushMany]
  | cons v rest ih =>
      calc
        (State.pushMany st (v :: rest)).stack.length
            = (State.pushMany (State.push1 st v) rest).stack.length := by simpa [State.pushMany]
        _ = (State.push1 st v).stack.length + rest.length := ih (State.push1 st v)
        _ = st.stack.length + 1 + rest.length := by simp [State.push1]
        _ = st.stack.length + (rest.length + 1) := by omega

@[simp] theorem setLocal_length (st : State) (idx : Nat) (v : Value) :
    (State.setLocal st idx v).stack.length = st.stack.length := by
  by_cases h : idx < st.locals.length
  · simp [State.setLocal, h]
  · simp [State.setLocal, h]

/-- The uniform pop-then-push length law (well-typed): after popping `p`
and pushing `vs` (of length `q`), `len' = len - p + q`. -/
theorem pop_push_length (st : State) (p : Nat) (vs : List Value) (q : Nat)
    (h : p ≤ st.stack.length) (hv : vs.length = q) :
    (State.pushMany (State.popN st p).1 vs).stack.length = st.stack.length - p + q := by
  have hp := State.popN_length_ge st p h
  rw [pushMany_length (State.popN st p).1 vs, hp, hv]

end State

/-- A run outcome: `ok` with the successor state, or `trap` (a retained
check failed and control stopped — T-C's "terminating executions" are the
`ok` runs). -/
inductive Outcome where
  | ok (st : State)
  | trap
  deriving Repr, Inhabited

/-- The concrete memory: recorded point stores plus the two oracles (§Q13 —
nondeterminism is a parameter of the semantics, never a fixed value). -/
structure ConcreteMem where
  cells : List (Int × Value)
  loadOracle : Int → Value
  mmioOracle : String → Value
  deriving Inhabited

namespace ConcreteMem

def empty (lo : Int → Value) (mo : String → Value) : ConcreteMem :=
  { cells := [], loadOracle := lo, mmioOracle := mo }

def record (m : ConcreteMem) (addr : Value) (val : Value) : ConcreteMem :=
  { m with cells := (addr, val) :: m.cells }

/-- Concrete load: a recorded point store wins; everything else is the
oracle's choice (unmodeled memory, §Q13). -/
def load (m : ConcreteMem) (addr : Value) : Value :=
  match m.cells.find? (fun c => c.1 == addr) with
  | some (_, v) => v
  | none => m.loadOracle addr

/-- Concrete MMIO read: the oracle's choice (the width bound is the
runtime's contract; T-C only needs the stack effect). -/
def mmioRead (m : ConcreteMem) : Value :=
  m.mmioOracle ""

end ConcreteMem

/-- A concrete op: the form plus the payload the step consults. -/
structure ConcreteOp where
  form : Tyu.IR.OpForm
  constVal : Option Int
  constBool : Option Bool
  slot : Option Nat
  subtypeCast : Bool
  brTgt : Option Nat
  brIfTgts : Option (Nat × Nat)
  callSig : Option (Nat × Nat)
  deriving DecidableEq, Repr, Inhabited

namespace ConcreteOp

def opMk (form : Tyu.IR.OpForm) : ConcreteOp :=
  { form := form, constVal := none, constBool := none, slot := none,
    subtypeCast := false, brTgt := none, brIfTgts := none, callSig := none }

def setConst (o : ConcreteOp) (v : Int) : ConcreteOp := { o with constVal := some v }
def setConstBool (o : ConcreteOp) (b : Bool) : ConcreteOp := { o with constBool := some b }
def setSlot (o : ConcreteOp) (i : Nat) : ConcreteOp := { o with slot := some i }
def setSubtypeCast (o : ConcreteOp) (b : Bool) : ConcreteOp := { o with subtypeCast := b }
def setBrTgt (o : ConcreteOp) (i : Nat) : ConcreteOp := { o with brTgt := some i }
def setBrIf (o : ConcreteOp) (p : Nat × Nat) : ConcreteOp := { o with brIfTgts := some p }
def setCallSig (o : ConcreteOp) (p : Nat × Nat) : ConcreteOp := { o with callSig := some p }

/-- The resolved call sig, defaulting to the representative (1,1). -/
def sig (o : ConcreteOp) : Nat × Nat :=
  o.callSig.getD (1, 1)

/-- The stack effect of `call`: the resolved sig's (out − in). -/
def callNet (o : ConcreteOp) : Int :=
  let (i, oo) := o.sig
  (oo : Int) - (i : Int)

/-- The *effective* pop count of an op: the table's pops for every form
except `call`, whose pop count is the resolved sig's (the underflow guard
of `stepOk` uses this so a successful call is a feasible call). -/
def effectivePops (o : ConcreteOp) : Nat :=
  match o.form with
  | .call => (o.callSig.getD (1, 1)).1
  | f => Tyu.IR.OpForm.pops f

/-- The stack effect of the op: `pushes − pops`. For `call` this is the
sig-accurate `callNet`; for every other form it is the generated table's
value. -/
def net (o : ConcreteOp) : Int :=
  match o.form with
  | .call => o.callNet
  | f => Tyu.IR.OpForm.net f

end ConcreteOp

/-- The concrete step: pops the form's table-pop count, computes the
successor values, pushes them. `Outcome.trap` is produced exactly at
`trap_if_false` with a false condition (a retained check). -/
def stepOp (_spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (o : ConcreteOp) (st : State) : ConcreteMem × Outcome :=
  match o.form with
  | .const_i64 => (mem, .ok (State.push1 st (o.constVal.getD 0)))
  | .const_bool => (mem, .ok (State.push1 st (if o.constBool.getD false then 1 else 0)))
  | .const_str => (mem, .ok (State.push1 st 0))
  | .addr_of | .addr_of_mut | .mmio_place | .scoped_enter | .task_spawn =>
      (mem, .ok (State.push1 st 0))
  | .ptr_add_const =>
      let (s1, _) := State.pop1 st
      (mem, .ok (State.push1 s1 0))
  | .ptr_add_index =>
      let (s2, _, _) := State.pop2 st
      (mem, .ok (State.push1 s2 0))
  | .dup =>
      let (s1, v) := State.pop1 st
      (mem, .ok (State.pushMany s1 [v, v]))
  | .drop =>
      let (s1, _) := State.pop1 st
      (mem, .ok s1)
  | .swap =>
      let (s1, b) := State.pop1 st
      let (s2, a) := State.pop1 s1
      (mem, .ok (State.pushMany s2 [b, a]))
  | .add_i64 =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (wrapI64 (a + b))))
  | .sub_i64 =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (wrapI64 (a - b))))
  | .mul_i64 =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (wrapI64 (a * b))))
  | .cmp_lt | .cmp_le | .cmp_gt | .cmp_ge | .cmp_eq | .cmp_ne =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (if cmpKind o.form a b then 1 else 0)))
  | .and_bool =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (if a ≠ 0 && b ≠ 0 then 1 else 0)))
  | .or_bool =>
      let (s2, a, b) := State.pop2 st
      (mem, .ok (State.push1 s2 (if a ≠ 0 || b ≠ 0 then 1 else 0)))
  | .not_bool =>
      let (s1, v) := State.pop1 st
      (mem, .ok (State.push1 s1 (if v == 0 then 1 else 0)))
  | .interrupt_disable | .interrupt_enable => (mem, .ok st)
  | .local_set =>
      let (s1, v) := State.pop1 st
      (mem, .ok (State.setLocal s1 (o.slot.getD 0) v))
  | .local_get =>
      (mem, .ok (State.push1 st (st.locals.getD (o.slot.getD 0) 0)))
  | .cast | .bitcast =>
      let (s1, v) := State.pop1 st
      (mem, .ok (State.push1 s1 v))
  | .call =>
      let (i, oo) := o.sig
      let (s1, _) := State.popN st i
      (mem, .ok (State.pushMany s1 (List.replicate oo 0)))
  | .load =>
      let (s1, addr) := State.pop1 st
      (mem, .ok (State.push1 s1 (mem.load addr)))
  | .store =>
      let (s1, v) := State.pop1 st
      let (s2, addr) := State.pop1 s1
      (mem.record addr v, .ok s2)
  | .vol_load | .vol_load_field =>
      let (s1, _) := State.pop1 st
      (mem, .ok (State.push1 s1 mem.mmioRead))
  | .vol_store | .vol_store_field =>
      let (s1, _) := State.pop1 st
      let (s2, _) := State.pop1 s1
      (mem, .ok s2)
  | .trap_if_false =>
      let (s1, v) := State.pop1 st
      if v == 0 then (mem, .trap) else (mem, .ok s1)
  | .br_if =>
      let (s1, _) := State.pop1 st
      (mem, .ok s1)
  | .br | .ret => (mem, .ok st)
where
  cmpKind : Tyu.IR.OpForm → Value → Value → Bool
    | .cmp_lt, a, b => a < b
    | .cmp_le, a, b => a ≤ b
    | .cmp_gt, a, b => a > b
    | .cmp_ge, a, b => a ≥ b
    | .cmp_eq, a, b => a == b
    | .cmp_ne, a, b => a ≠ b
    | _, _, _ => false

/-- Run a sequence of ops; a trap terminates the run. -/
def runOps (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (ops : List ConcreteOp) (st : State) : ConcreteMem × Outcome :=
  ops.foldl
    (fun (acc : ConcreteMem × Outcome) o =>
      match acc.2 with
      | Outcome.ok s => stepOp spec acc.1 o s
      | Outcome.trap => acc)
    (mem, Outcome.ok st)

/-- The ok-state projection of `stepOp`, with the underflow guard: `none`
when the op would pop more than the stack holds (a well-typed program never
does; modeling it as a failed step keeps T-C's per-op length law
premise-free). `none` also covers the retained-check trap. -/
def stepOk (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (o : ConcreteOp) (st : State) : Option State :=
  if ConcreteOp.effectivePops o > st.stack.length then
    none
  else
    match (stepOp spec mem o st).2 with
    | Outcome.ok st' => some st'
    | Outcome.trap => none

/-- The block structure: id and ops (the terminator is the last op). -/
structure Block where
  id : Nat
  ops : List ConcreteOp
  deriving Repr, Inhabited

namespace Block

def empty : Block := { id := 0, ops := [] }

/-- A block's end: the routing decision the run makes after the body. -/
inductive End where
  | ret (st : State)
  | go (tgt : Nat) (st : State)
  | brIf (thenTgt : Nat) (elseTgt : Nat) (cond : Value) (st : State)
  | trap
  deriving Repr, Inhabited

/-- Step one block: fold its ops; when a control op is reached the block
ends (for `br_if` the condition is popped — the br_if stack effect — and
carried in the end for routing). A trap ends the block without a successor.
Structural recursion over the op list (equivalent to the earlier
`let rec` form — same semantics, but definitionally transparent so the
statement computations and worked proofs reduce it directly). -/
def runBlock (spec : Tyu.IR.TargetSpec) (mem : ConcreteMem) (ops : List ConcreteOp) (st : State) : ConcreteMem × End :=
  match ops with
  | [] => (mem, .ret st)
  | o :: rest =>
      match o.form with
      | .ret => (mem, .ret st)
      | .br => (mem, .go (o.brTgt.getD 0) st)
      | .br_if =>
          let (s1, v) := State.pop1 st
          let (t, e) := o.brIfTgts.getD (0, 0)
          (mem, .brIf t e v s1)
      | _ =>
          match stepOp spec mem o st with
          | (m1, .trap) => (m1, .trap)
          | (m1, .ok s1) => runBlock spec m1 rest s1

end Block

/-- The word-level run: fuel steps through the CFG from block `entry`;
`fuel = 0` means the run did not terminate within the budget. -/
def runWord (blocks : List Block) (spec : Tyu.IR.TargetSpec) (entry : Nat) (fuel : Nat)
    (mem : ConcreteMem) (st : State) : ConcreteMem × Option State :=
  match fuel with
  | 0 => (mem, none)
  | fuel' + 1 =>
      let b := blocks.getD entry Block.empty
      match Block.runBlock spec mem b.ops st with
      | (mem1, .ret st1) => (mem1, some st1)
      | (mem1, .go t st1) => runWord blocks spec t fuel' mem1 st1
      | (mem1, .brIf t e cond st1) =>
          if cond ≠ 0 then runWord blocks spec t fuel' mem1 st1
          else runWord blocks spec e fuel' mem1 st1
      | (mem1, .trap) => (mem1, none)

/-- Termination: a word run that completes within the fuel budget. -/
def Terminates (blocks : List Block) (spec : Tyu.IR.TargetSpec) (entry : Nat) (fuel : Nat)
    (mem : ConcreteMem) (st : State) : Prop :=
  ∃ st', runWord blocks spec entry fuel mem st = (mem, some st')

end Tyu.Step