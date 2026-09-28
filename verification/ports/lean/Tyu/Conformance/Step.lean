import Tyu.IR.Op
import Tyu.Conformance.Interval

namespace Tyu.Conformance

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
  deriving DecidableEq, Repr, Inhabited

namespace MemModel

/-- Is a point address inside the modeled RAM window? -/
def inRam (m : MemModel) (a : Int) : Bool :=
  match m with
  | .bundle ramLo ramHi _ => ramLo ≤ a && a ≤ ramHi
  | _ => false

/-- The recorded-cell list (empty for the non-bundle instances). -/
def cellsOf (m : MemModel) : List (Int × Interval) :=
  match m with
  | .bundle _ _ cs => cs
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
        | .bundle ramLo ramHi cs =>
            if ramLo ≤ a && a ≤ ramHi then .bundle ramLo ramHi (storeCells cs a val) else m
        | _ => m
      else m
  | _ => m

/-- MMIO aperture read — the injected nondeterminism oracle (§Q13). `flat`
answers `top` (sound for any register width); the aperture-bearing
instances answer the width-bounded domain. -/
def apertureRead (m : MemModel) (widthBits : Nat) : Interval :=
  match m with
  | .flat => Interval.top
  | .apertureEmpty | .bundle _ _ _ =>
      match wordDomain widthBits with
      | none => Interval.top
      | some (lo, hi) => Interval.range lo hi

end MemModel

/-- One parsed op instance: the op form plus the payload fields the abstract
transfer consumes. -/
structure OpInst where
  form : Tyu.IR.OpForm
  constVal : Option Int
  constBool : Option Bool
  slot : Option Nat
  subtypeCast : Bool
  brTgt : Option Nat
  brIfTgts : Option (Nat × Nat)
  deriving DecidableEq, Repr, Inhabited

namespace OpInst

/-- The empty/default instance (all payload fields unset). Named `opMk` to
avoid colliding with the structure's generated `OpInst.mk` constructor. -/
def opMk (form : Tyu.IR.OpForm) : OpInst :=
  { form := form, constVal := none, constBool := none, slot := none,
    subtypeCast := false, brTgt := none, brIfTgts := none }

/-- Payload setters (projection-style helpers for the parser). -/
def setConst (o : OpInst) (v : Int) : OpInst := { o with constVal := some v }
def setConstBool (o : OpInst) (b : Bool) : OpInst := { o with constBool := some b }
def setSlot (o : OpInst) (i : Nat) : OpInst := { o with slot := some i }
def setSubtypeCast (o : OpInst) (b : Bool) : OpInst := { o with subtypeCast := b }
def setBrTgt (o : OpInst) (i : Nat) : OpInst := { o with brTgt := some i }
def setBrIf (o : OpInst) (p : Nat × Nat) : OpInst := { o with brIfTgts := some p }

end OpInst

/-- The subtype range table the corpus's cast vectors reference: the
`synthetic-word` subtype `percent 0..=100` (TypeId 2 in the vector corpus's
word). -/
def percentRange : Option (Int × Int) := some (0, 100)

/-- The normative transfer over one op (`verifier::interp::State::step`,
parameterized by target word width and memory model). Returns the updated
model alongside the state (P12.2: the bundle instance's recorded stores are
observable — `store` mutates the model, `load`/`vol_load` consult it). -/
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
      (mem, st1.push' (Slot.computed (mem.apertureRead widthBits)))
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

end Tyu.Conformance
