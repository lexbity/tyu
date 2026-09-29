import Tyu.IR.Op
import Std

/-! THE abstract interval implementation (PLAN-VERIFY-3 P14.1, §Q12/§Q16).

`Tyu.Abs` is **the port's single interval implementation** — the lattice
domain, the abstract state, and the abstract transfer (`stepOp`/`absRun`)
absorbed from `Tyu/Conformance/Interval.lean` and
`Tyu/Conformance/Step.lean` (P4-audit consolidation, 2026-09-26: the port
must not carry two interval layers that can drift). The conformance
`Tyu/Conformance/*` files re-export these definitions by `abbrev`; the
abstract engine the corpus runner, the stackmeta path, and the `rederive`
exe (P14.2) all execute **this one** implementation.

Mirrors `verifier::interval` (the Rust lattice) and
`verifier::interp::State::step` (the Rust abstract transfer): soundness of
the in-tree discharge rests on the T-A/T-B registry statements
(`Tyu.Sound.TA`/`Tyu.Sound.TB`), proved over this file — the kernel
vouches for the *math*, the conformance vectors pin the *data*.

The transfer table is rendered like the concrete table: `stepOp` matches
WILDCARD-FREE on the generated `Tyu.IR.OpForm` — a semantics row added
without an abstract-transfer arm is a port compile error (FR-12, the same
discipline as `Tyu/Step.lean`). -/

namespace Tyu.Abs

/-- The abstract interval lattice: bottom (empty), a closed range on i64,
top (whole i64 domain). Mirrors `verifier::interval::Interval`. -/
inductive Interval where
  | bottom
  | top
  | range (lo : Int) (hi : Int)
  deriving DecidableEq, Repr, Inhabited

namespace Interval

def isBottom : Interval → Bool
  | .bottom => true
  | _ => false

def isTop : Interval → Bool
  | .top => true
  | _ => false

def join : Interval → Interval → Interval
  | .bottom, x => x
  | x, .bottom => x
  | .top, _ => .top
  | _, .top => .top
  | .range a b, .range c d => .range (min a c) (max b d)

def subsetOf : Interval → Interval → Bool
  | .bottom, _ => true
  | _, .top => true
  | .range a b, .range c d => c ≤ a && b ≤ d
  | _, _ => false

/-- i64 domain bounds. -/
def i64Min : Int := -9223372036854775808
def i64Max : Int := 9223372036854775807

def inI64Domain (x : Int) : Bool := i64Min ≤ x && x ≤ i64Max

/-- The concretization predicate ($T-A's bridge): `x` is a concrete value
the abstract interval represents. `bottom` represents nothing; `top`
represents every i64 value; `range lo hi` the closed range. -/
def Contains (iv : Interval) (x : Int) : Prop :=
  match iv with
  | .bottom => False
  | .top => True
  | .range lo hi => lo ≤ x ∧ x ≤ hi

/-- add with the i64-with-wrap-to-top rule (`verifier::interval::Interval::add`). -/
def add : Interval → Interval → Interval
  | .bottom, _ | _, .bottom => .bottom
  | .top, _ | _, .top => .top
  | .range a b, .range c d =>
      let lo := a + c
      let hi := b + d
      if inI64Domain lo && inI64Domain hi then .range lo hi else .top

/-- sub (`verifier::interval::Interval::sub`). -/
def sub : Interval → Interval → Interval
  | .bottom, _ | _, .bottom => .bottom
  | .top, _ | _, .top => .top
  | .range a b, .range c d =>
      let lo := a - d
      let hi := b - c
      if inI64Domain lo && inI64Domain hi then .range lo hi else .top

/-- mul by the four-corner tableau (`verifier::interval::Interval::mul`). -/
def mul : Interval → Interval → Interval
  | .bottom, _ | _, .bottom => .bottom
  | .top, _ | _, .top => .top
  | .range a b, .range c d =>
      let w := a * c
      let x := a * d
      let y := b * c
      let z := b * d
      let lo := min (min w x) (min y z)
      let hi := max (max w x) (max y z)
      if inI64Domain w && inI64Domain x && inI64Domain y && inI64Domain z
        then .range lo hi else .top

/-- Narrowing cast meet: iv ∩ [lo, hi]. -/
def castNarrow (iv : Interval) (lo hi : Int) : Interval :=
  match iv with
  | .bottom => .bottom
  | .top => .range lo hi
  | .range a b =>
      let lo' := max a lo
      let hi' := min b hi
      if lo' ≤ hi' then .range lo' hi' else .bottom

/-- The back-edge widening operator `∇` (§7.2/FR-12; `v ∇ old = old when
`new ⊑ old`, else `⊤`): loop-carried slots reach `⊤` in ≤ 2 passes. Sound —
widening only GROWS the abstract value, so a discharge over the widened
value is a discharge over every pre-widening value (`Tyu.Sound.TA.widen_sound`). -/
def widenOld (old new : Interval) : Interval :=
  if new.subsetOf old then old else .top

end Interval

/-- The three-valued obligation head (mirrors `verifier::interval::Tri`). -/
inductive Tri where
  | defTrue | defFalse | top
  deriving DecidableEq, Repr, Inhabited

namespace Tri

def name : Tri → String
  | .defTrue => "discharged"
  | .defFalse => "def-false"
  | .top => "open"

end Tri

/-- `eval_in_range` (`verifier::interval::eval_in_range`). -/
def evalInRange (iv : Interval) (lo hi : Int) : Tri :=
  match iv with
  | .bottom => Tri.top
  | .top => Tri.top
  | .range a b =>
      if a ≥ lo && b ≤ hi then Tri.defTrue
      else if b < lo || a > hi then Tri.defFalse
      else Tri.top

/-- tri_from_bool_iv. -/
def triFromBoolIv (iv : Interval) : Tri :=
  match iv with
  | .range lo hi => if lo == 0 && hi == 0 then Tri.defFalse
                    else if lo == 1 && hi == 1 then Tri.defTrue
                    else Tri.top
  | _ => Tri.top

/-- bool_iv. -/
def boolIv : Tri → Interval
  | .defTrue => .range 1 1
  | .defFalse => .range 0 0
  | .top => .range 0 1

def triNot : Tri → Tri
  | .defTrue => .defFalse
  | .defFalse => .defTrue
  | .top => .top

def triAnd : Tri → Tri → Tri
  | .defTrue, x => x
  | x, .defTrue => x
  | .defFalse, _ => .defFalse
  | _, .defFalse => .defFalse
  | .top, .top => .top

def triOr : Tri → Tri → Tri
  | .defFalse, x => x
  | x, .defFalse => x
  | .defTrue, _ => .defTrue
  | _, .defTrue => .defTrue
  | .top, .top => .top

/-- The abstract comparison `a rel b` (`verifier::interval::tri_cmp`). -/
def triCmp (a b : Interval) (kind : String) : Tri :=
  match a, b with
  | .bottom, _ | _, .bottom => Tri.top
  | .top, _ | _, .top => Tri.top
  | .range x y, .range u v =>
      match kind with
      | "cmp_lt" => if y < u then Tri.defTrue else if x ≥ v then Tri.defFalse else Tri.top
      | "cmp_le" => if y ≤ u then Tri.defTrue else if x > v then Tri.defFalse else Tri.top
      | "cmp_gt" => if x > v then Tri.defTrue else if y ≤ u then Tri.defFalse else Tri.top
      | "cmp_ge" => if x ≥ v then Tri.defTrue else if y < u then Tri.defFalse else Tri.top
      | "cmp_eq" =>
          if x == y && u == v && x == u then Tri.defTrue
          else if y < u || x > v then Tri.defFalse
          else Tri.top
      | "cmp_ne" =>
          if x == y && u == v && x == u then Tri.defFalse
          else if y < u || x > v then Tri.defTrue
          else Tri.top
      | _ => Tri.top

/-- The CONCRETE comparison result (`verifier::concrete` comparison row): the
0/1 the runtime's `cmp_*` pushes for two concrete i64 values. T-A's
comparison-soundness theorem pairs it with [`triCmp`]: the abstract bool
interval contains the concrete result for any concretization. -/
def concreteCmp (kind : String) (x y : Int) : Bool :=
  match kind with
  | "cmp_lt" => x < y
  | "cmp_le" => x ≤ y
  | "cmp_gt" => x > y
  | "cmp_ge" => x ≥ y
  | "cmp_eq" => x == y
  | _ => x ≠ y

/-- the width-relative word domain (`verifier::interval::word_domain`),
rendered as `none` for the full i64 domain. -/
def wordDomain (bits : Nat) : Option (Int × Int) :=
  let b := if bits == 0 then 64 else bits
  if b ≥ 64 then none
  else
    let half := (2 : Int) ^ (b - 1)
    some (-half, half - 1)

/-- The text form of an interval (matches `tyu.vec/1` expect.top strings). -/
def toText (iv : Interval) : String :=
  match iv with
  | .bottom => "<bottom>"
  | .top => "<top>"
  | .range lo hi => "[" ++ toString lo ++ "," ++ toString hi ++ "]"

-- -----------------------------------------------------------------------
-- The abstract state + transfer (absorbed from `Conformance/Step.lean`).
-- -----------------------------------------------------------------------

/-- Map a function with the element index over a list (small core-only
equivalent of `List.mapIdx`). -/
def mapIdx {α : Type} (l : List α) (f : Nat → α → α) : List α :=
  let rec go : Nat → List α → List α
    | _, [] => []
    | i, x :: xs => f i x :: go (i + 1) xs
  go 0 l

/-- Max abstract stack/local depth (mirrors `verifier::interp::MAX_SLOTS`). -/
def MAX_SLOTS : Nat := 64

/-- One abstract stack/local slot carrying an interval (the identity lattice
is not observed by `tyu.vec/1`, so it is omitted here). -/
structure Slot where
  iv : Interval
  deriving DecidableEq, Repr, Inhabited

namespace Slot

def top : Slot := { iv := Interval.top }
def computed (iv : Interval) : Slot := { iv := iv }

end Slot

structure State where
  stack : List Slot
  locals : List Slot
  deriving DecidableEq, Repr, Inhabited

namespace State

/-- A fresh state: empty stack, `localsCap` top locals. -/
def fresh (localsCap : Nat) : State :=
  { stack := [], locals := List.replicate (min localsCap MAX_SLOTS) Slot.top }

/-- The callee-entry state: `inputs` top/arg slots on the stack (interval-wise
top; exact `verifier::interp::State::callee_entry`). -/
def calleeEntry (inputs : Nat) (localsCap : Nat) : State :=
  { stack := List.replicate inputs Slot.top, locals := List.replicate (min localsCap MAX_SLOTS) Slot.top }

/-- Rust push: if the stack is already at the cap, truncate to MAX_SLOTS-1
elements before pushing (so the post-push length is MAX_SLOTS). -/
def push' (st : State) (s : Slot) : State :=
  let stack' := if st.stack.length ≥ MAX_SLOTS then List.drop (st.stack.length - (MAX_SLOTS - 1)) st.stack else st.stack
  { st with stack := stack' ++ [s] }

/-- Simpler popVal: the top slot is the last element. -/
def popVal' (st : State) : State × Slot :=
  match st.stack.reverse with
  | [] => (st, Slot.top)
  | top :: rest => ({ st with stack := rest.reverse }, top)

def pop2 (st : State) : State × Slot × Slot :=
  let (st1, second) := popVal' st
  let (st2, first) := popVal' st1
  (st2, first, second)

def binop (st : State) (f : Interval → Interval → Interval) : State :=
  let (st2, first, second) := pop2 st
  st2.push' (Slot.computed (f first.iv second.iv))

def topInterval (st : State) : Interval :=
  match st.stack.reverse with
  | [] => Interval.bottom
  | top :: _ => top.iv

/-- The stack depth at which a branch entry state is truncated. -/
def truncateStack (st : State) (len : Nat) : State :=
  if st.stack.length > len then { st with stack := List.drop (st.stack.length - len) st.stack } else st

end State

/-- The memory model boundary (P3.2's degenerate instances + the P12.2
bundle instance). `flat` = FlatMem (unmodeled memory, reads `top`);
`apertureEmpty` = an empty ApertureMem (no RAM); `bundle` = ApertureMem
over the bundle's modeled RAM window — the **inclusive** `[ramLo, ramHi]`
pair from the corpus `"ram"` header (`model/model.toml [memory] ram`'s
half-open region minus its exclusive top byte), mirroring
`verifier::mem::ApertureMem`: recorded point stores within the window are
returned by loads; everything else is `top`. -/
inductive MemModel where
  | flat
  | apertureEmpty
  | bundle (ramLo : Int) (ramHi : Int) (cells : List (Int × Interval))
           (scripted : List (String × Interval))
  deriving DecidableEq, Repr, Inhabited

namespace MemModel

/-- Is a point address inside the modeled RAM window? -/
def inRam (m : MemModel) (a : Int) : Bool :=
  match m with
  | .bundle ramLo ramHi _ _ => ramLo ≤ a && a ≤ ramHi
  | _ => false

/-- The recorded-cell list (empty for the non-bundle instances). -/
def cellsOf (m : MemModel) : List (Int × Interval) :=
  match m with
  | .bundle _ _ cs _ => cs
  | _ => []

/-- The joined abstract value recorded for a point address (`top` when
unrecorded — mirroring `ApertureMem::recorded_at`; with replace-semantics
stores there is at most one live cell per address). -/
def recordedAt (m : MemModel) (a : Int) : Interval :=
  match m.cellsOf.find? (fun c => c.1 == a) with
  | some (_, v) => v
  | none => Interval.top

/-- Store with replace semantics (`ApertureMem::store`): the newest value
for an address replaces any prior cell. Head-first so `find?` sees the
newest first. -/
def storeCells (cs : List (Int × Interval)) (a : Int) (v : Interval) : List (Int × Interval) :=
  (a, v) :: cs.filter (fun c => c.1 ≠ a)

/-- Abstract load (`verifier::mem::MemModel::load`): a point address within
the RAM window answers the recorded join; a non-point or unmapped address is
`top` (it could alias anything). -/
def load (m : MemModel) (addr : Interval) (_widthBits : Nat) : Interval :=
  match addr with
  | Interval.range a b => if a == b && m.inRam a then m.recordedAt a else Interval.top
  | _ => Interval.top

/-- Abstract store: only point addresses within the modeled RAM window are
tracked; anything else keeps reads at `top` (sound — the load
over-approximates). -/
def store (m : MemModel) (addr : Interval) (val : Interval) : MemModel :=
  match addr with
  | Interval.range a b =>
      if a == b then
        match m with
        | .bundle ramLo ramHi cs scr =>
            if ramLo ≤ a && a ≤ ramHi then .bundle ramLo ramHi (storeCells cs a val) scr else m
        | _ => m
      else m
  | _ => m

/-- The scripted (refined) read for `place`, when the model declares one —
mirrors `ApertureMem::script_read` (the refinement oracle seed). -/
def scriptedRead (m : MemModel) (place : String) : Option Interval :=
  match m with
  | .bundle _ _ _ scr =>
      match scr.find? (fun s => s.1 == place) with
      | some (_, v) => some v
      | none => none
  | _ => none

/-- MMIO aperture read — the injected nondeterminism oracle (§Q13). A
`bundle`-instance read of a SCRIPTED (refined) `place` answers the scripted
value (the refinement's modeled behavior); every other read answers `top`
(`flat`) or the width-bounded domain. -/
def apertureRead (m : MemModel) (place : String) (widthBits : Nat) : Interval :=
  match m with
  | .flat => Interval.top
  | .apertureEmpty =>
      match wordDomain widthBits with
      | none => Interval.top
      | some (lo, hi) => Interval.range lo hi
  | .bundle _ _ _ _ =>
      match m.scriptedRead place with
      | some v => v
      | none =>
          match wordDomain widthBits with
          | none => Interval.top
          | some (lo, hi) => Interval.range lo hi

end MemModel

/-- One parsed op instance: the op form plus the payload fields the abstract
transfer consumes. `place` is the MMIO register token of a
`vol_load`/`vol_store`/`addr_of`/`mmio_place` access (P13.2: the refined-
evidence corpus scripts reads by place, mirroring
`ApertureMem.script_read`). -/
structure OpInst where
  form : Tyu.IR.OpForm
  constVal : Option Int
  constBool : Option Bool
  slot : Option Nat
  subtypeCast : Bool
  brTgt : Option Nat
  brIfTgts : Option (Nat × Nat)
  place : String
  deriving DecidableEq, Repr, Inhabited

namespace OpInst

/-- The empty/default instance (all payload fields unset). Named `opMk` to
avoid colliding with the structure's generated `OpInst.mk` constructor. -/
def opMk (form : Tyu.IR.OpForm) : OpInst :=
  { form := form, constVal := none, constBool := none, slot := none,
    subtypeCast := false, brTgt := none, brIfTgts := none, place := "" }

/-- Payload setters (projection-style helpers for the parser). -/
def setConst (o : OpInst) (v : Int) : OpInst := { o with constVal := some v }
def setConstBool (o : OpInst) (b : Bool) : OpInst := { o with constBool := some b }
def setSlot (o : OpInst) (i : Nat) : OpInst := { o with slot := some i }
def setSubtypeCast (o : OpInst) (b : Bool) : OpInst := { o with subtypeCast := b }
def setBrTgt (o : OpInst) (i : Nat) : OpInst := { o with brTgt := some i }
def setBrIf (o : OpInst) (p : Nat × Nat) : OpInst := { o with brIfTgts := some p }
def setPlace (o : OpInst) (p : String) : OpInst := { o with place := p }

end OpInst

/-- The subtype range table the corpus's cast vectors reference: the
`synthetic-word` subtype `percent 0..=100` (TypeId 2 in the vector corpus's
word). -/
def percentRange : Option (Int × Int) := some (0, 100)

/-- The normative abstract transfer over one op (`verifier::interp::State::step`,
parameterized by target word width and memory model). Returns the updated
model alongside the state (P12.2: the bundle instance's recorded stores are
observable — `store` mutates the model, `load`/`vol_load` consult it). The
match over `op.form : Tyu.IR.OpForm` is WILDCARD-FREE — a semantics row
added without an abstract-transfer arm is a port compile error (the
"rendered like the concrete table" guarantee). -/
def stepOp (op : OpInst) (st : State) (sr : Option (Int × Int)) (widthBits : Nat) (mem : MemModel) : MemModel × State :=
  match op.form with
  | .const_i64 =>
      let v := op.constVal.getD 0
      (mem, st.push' (Slot.computed (Interval.range v v)))
  | .const_bool =>
      let b := op.constBool.getD false
      (mem, st.push' (Slot.computed (Interval.range (if b then 1 else 0) (if b then 1 else 0))))
  | .const_str => (mem, st.push' Slot.top)
  | .addr_of | .addr_of_mut | .mmio_place | .scoped_enter | .task_spawn => (mem, st.push' Slot.top)
  | .ptr_add_const =>
      let (st1, _) := State.popVal' st
      (mem, st1.push' Slot.top)
  | .ptr_add_index =>
      let (st1, _, _) := State.pop2 st
      (mem, st1.push' Slot.top)
  | .dup =>
      let (st1, v) := State.popVal' st
      (mem, st1.push' v |>.push' v)
  | .drop =>
      let (st1, _) := State.popVal' st
      (mem, st1)
  | .swap =>
      let (st1, a, b) := State.pop2 st
      (mem, st1.push' b |>.push' a)
  | .add_i64 => (mem, State.binop st Interval.add)
  | .sub_i64 => (mem, State.binop st Interval.sub)
  | .mul_i64 => (mem, State.binop st Interval.mul)
  | .cmp_lt | .cmp_le | .cmp_gt | .cmp_ge | .cmp_eq | .cmp_ne =>
      let (st1, a, b) := State.pop2 st
      let t := triCmp a.iv b.iv op.form.mnemonic
      (mem, st1.push' (Slot.computed (boolIv t)))
  | .and_bool =>
      let (st1, a, b) := State.pop2 st
      let t := triAnd (triFromBoolIv a.iv) (triFromBoolIv b.iv)
      (mem, st1.push' (Slot.computed (boolIv t)))
  | .or_bool =>
      let (st1, a, b) := State.pop2 st
      let t := triOr (triFromBoolIv a.iv) (triFromBoolIv b.iv)
      (mem, st1.push' (Slot.computed (boolIv t)))
  | .not_bool =>
      let (st1, a) := State.popVal' st
      let t := triNot (triFromBoolIv a.iv)
      (mem, st1.push' (Slot.computed (boolIv t)))
  | .interrupt_disable | .interrupt_enable => (mem, st)
  | .local_set =>
      let (st1, v) := State.popVal' st
      let idx := op.slot.getD 0
      let locals' := if idx < st1.locals.length
        then (mapIdx st1.locals (fun i s => if i == idx then v else s))
        else st1.locals
      (mem, { st1 with locals := locals' })
  | .local_get =>
      let idx := op.slot.getD 0
      let v := st.locals.getD idx Slot.top
      (mem, st.push' v)
  | .cast =>
      -- narrowing cast to the subtype: the abstract successor is the
      -- intersection with the target range; otherwise identity.
      let (st1, v) := State.popVal' st
      let iv := match sr with
        | some (lo, hi) => if op.subtypeCast then Interval.castNarrow v.iv lo hi else v.iv
        | none => v.iv
      (mem, st1.push' (Slot.computed iv))
  | .bitcast =>
      let (st1, v) := State.popVal' st
      (mem, st1.push' v)
  | .call =>
      -- representative call sig (1,1): row pops/pushes (drives the corpus
      -- vectors; a real call's sig is payload-carried in the full pipeline).
      let (st1, _) := State.popVal' st
      (mem, st1.push' Slot.top)
  | .load =>
      let (st1, addr) := State.popVal' st
      (mem, st1.push' (Slot.computed (mem.load addr.iv widthBits)))
  | .store =>
      let (st1, addr, val) := State.pop2 st
      (mem.store addr.iv val.iv, st1)
  | .vol_load | .vol_load_field =>
      let (st1, _) := State.popVal' st
      (mem, st1.push' (Slot.computed (mem.apertureRead op.place widthBits)))
  | .vol_store | .vol_store_field =>
      let (st1, _, _) := State.pop2 st
      (mem, st1)
  | .trap_if_false =>
      let (st1, _) := State.popVal' st
      (mem, st1)
  | .br_if =>
      let (st1, _) := State.popVal' st
      (mem, st1)
  | .br | .ret => (mem, st)

/-- The straight-line abstract run: fold `stepOp` over the op list, from a
given state/memory (the `absRun` the P14 `rederive` engine and the T-A/T-B
run-level statements execute). -/
def absRun (ops : List OpInst) (st : State) (sr : Option (Int × Int)) (widthBits : Nat) (mem : MemModel) : MemModel × State :=
  ops.foldl (fun (acc : MemModel × State) o =>
    let (m, s) := acc
    stepOp o s sr widthBits m) (mem, st)

end Tyu.Abs