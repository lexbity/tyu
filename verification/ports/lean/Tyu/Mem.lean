namespace Tyu.Mem

/- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by
   `crates/verifier/src/export/lean.rs` — DO NOT EDIT. The
   MemModel/DeviceModel/Services *interfaces* rendered from the verifier's
   memory-model boundary (`verifier::mem::MemModel`), with law-statement
   placeholders (P3.1: "GEN interfaces + law-statement placeholders"). The
   placeholders are structure fields — **data, not axioms** — so the library
   adds no axioms; the T-D memory-model laws are proven in Phase P12 over
   real bundle instances. -/
/-- The abstract value domain of the memory-model boundary (mirrors the
value lattice `Interval` of `verifier::interval`). -/
inductive IntervalVal where
  | bottom
  | top
  | range (lo : Int) (hi : Int)
  deriving DecidableEq, Repr, Inhabited

/-- The memory-model interface (`verifier::mem::MemModel`): an abstract
load/store and the MMIO aperture read/write oracle (§Q13: the aperture
read is the injected nondeterminism, width-bounded by the register). -/
structure MemModel where
  load : IntervalVal → Nat → IntervalVal
  store : IntervalVal → IntervalVal → IntervalVal
  apertureRead : String → Nat → IntervalVal
  apertureWrite : String → IntervalVal → IntervalVal
  deriving Inhabited

/-- Law-statement placeholders for the interface (the T-D family). These
are Prop fields of a structure parameterized by the model — data, not
axioms; Phase P12 states and proves the real laws. -/
structure MemModelLaws (M : MemModel) where
  loadSubsetTop : Prop
  storeLoadPoint : Prop
  apertureWidthBound : Prop
  deriving Inhabited

/-- The default model (`verifier::mem::FlatMem`): memory unmodeled, MMIO
reads entirely nondeterministic (top — sound for any register width). -/
def flatMem : MemModel := {
  load := fun _ _ => IntervalVal.top,
  store := fun _ v => v,
  apertureRead := fun _ _ => IntervalVal.top,
  apertureWrite := fun _ _ => IntervalVal.top }

end Tyu.Mem
