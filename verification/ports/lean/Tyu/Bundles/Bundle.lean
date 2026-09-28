import Tyu.Mem
import Tyu.Conformance.Interval
import Tyu.Conformance.Step

/-! The bundle memory instances (PLAN-VERIFY-3 P12.2).

Each modeled bare-metal bundle (`platforms/<triple>/model/`) declares a
model id and a `[memory] ram` window; the fixtures in this directory are
the abstract memory state the T-D subset laws are proved over. The substrate
is the *functional* model (`cell : Int → IntervalVal` — the denotational
shape the Rust `verifier::mem::ApertureMem`'s list-of-cells implements):
only point stores within the modeled window are observable by loads,
everything else reads `top`, and MMIO reads answer the §Q13 width-bounded
nondeterministic domain.

The conditions are **proof-carrying (`Prop`)** so the T-D laws are
kernel-checkable statements about the model; the operational mirror used by
the conformance runner is `Tyu.Conformance.MemModel.bundle` (Bool-based,
list cells) — the two agree on observables, pinned by the committed
`tyu.vec/1` bundle corpora executed by both this port and the Rust suite. -/

namespace Tyu.Bundles

open Tyu.Mem

/-- The abstract memory state of a modeled bundle. `cell` maps a point
address to its recorded abstract value (`top` = unrecorded/unmodeled); the
RAM window `[ramLo, ramHi]` (inclusive) is the only domain loads and
stores honor — mirroring `ApertureMem { ram, cells }`'s observable rules. -/
structure BundleMem where
  ramLo : Int
  ramHi : Int
  cell : Int → IntervalVal
  deriving Inhabited

namespace BundleMem

/-- Is a point address inside the modeled RAM window? A transparent `Prop`
(not a `Bool`) so the load/store branches are `dite`s (with `Decidable`
elaborating through the unfold) and the T-D laws close from their
hypotheses. -/
abbrev inRam (m : BundleMem) (a : Int) : Prop := m.ramLo ≤ a ∧ a ≤ m.ramHi

/-- The MMIO aperture read — the injected nondeterminism oracle (§Q13): a
value *within* the register's width, `top` at ≥ 64 bits (the full i64
domain). Defined through the **single** `Tyu.Conformance.wordDomain` — the
one width-domain implementation shared with the conformance runner and
`TargetSpec::mmio_read_domain` (verifier::interval::word_domain on the Rust
side), so the aperture paths literally cannot drift. Never wider than the
register. -/
def apertureRead (w : Nat) : IntervalVal :=
  match Tyu.Conformance.wordDomain w with
  | none => IntervalVal.top
  | some (lo, hi) => IntervalVal.range lo hi

/-- Abstract load (`verifier::mem::ApertureMem::load`): a point address in
the window answers the recorded cell; a non-point or unmapped address is
`top` (it could alias anything). -/
def loadVal (m : BundleMem) (addr : IntervalVal) : IntervalVal :=
  match addr with
  | IntervalVal.range a b =>
      if a = b then (if m.inRam a then m.cell a else IntervalVal.top) else IntervalVal.top
  | _ => IntervalVal.top

/-- Abstract store (`verifier::mem::ApertureMem::store`): a point address
in the window replaces its cell (newest wins — replace semantics, so a load
returns the last stored value, matching the Rust `retain + push`);
anything else is state-preserving (reads keep `top` — sound, the abstract
load over-approximates). -/
def storeVal (m : BundleMem) (addr : IntervalVal) (val : IntervalVal) : BundleMem :=
  match addr with
  | IntervalVal.range a b =>
      if a = b then
        if m.inRam a then { m with cell := fun a' => if a' = a then val else m.cell a' }
        else m
      else m
  | _ => m

end BundleMem

/-- The lattice coercion **to the conformance lattice** (`IntervalVal` →
`Tyu.Conformance.Interval`) — the two lattices are the same three-shape
lattice; this direction makes the operational `MemModel.bundle` and the
denotational `BundleMem` comparable (T-D equivalence below). -/
def toConformance : IntervalVal → Tyu.Conformance.Interval
  | IntervalVal.bottom => Tyu.Conformance.Interval.bottom
  | IntervalVal.top => Tyu.Conformance.Interval.top
  | IntervalVal.range a b => Tyu.Conformance.Interval.range a b

/-- The lattice coercion **from** the conformance lattice (the direction
used to build a `BundleMem` twin from an operational model's recorded
cells). -/
def fromConformance : Tyu.Conformance.Interval → IntervalVal
  | Tyu.Conformance.Interval.bottom => IntervalVal.bottom
  | Tyu.Conformance.Interval.top => IntervalVal.top
  | Tyu.Conformance.Interval.range a b => IntervalVal.range a b

theorem to_from_conformance (iv : Tyu.Conformance.Interval) :
    toConformance (fromConformance iv) = iv := by
  cases iv <;> rfl

/-- The `BundleMem` twin of an operational `MemModel.bundle`: the same
window, with the cell function reading exactly the conformance model's
recorded cells. -/
def ofCells (lo hi : Int) (cells : List (Int × Tyu.Conformance.Interval)) : BundleMem :=
  { ramLo := lo, ramHi := hi,
    cell := fun a =>
      fromConformance (Tyu.Conformance.MemModel.recordedAt
        (Tyu.Conformance.MemModel.bundle lo hi cells) a) }

/-- The twin's window-membership, as a proposition iff (the bridge the load
equivalence uses). -/
theorem ofCells_inRam_iff (lo hi : Int) (cells : List (Int × Tyu.Conformance.Interval)) (a : Int) :
    (ofCells lo hi cells).inRam a ↔ lo ≤ a ∧ a ≤ hi := by
  simp [ofCells, BundleMem.inRam]

/-- The window-membership bridge: the conformance model's Bool window test
equals the denotational window proposition. -/
theorem bundle_inRam_iff (lo hi a : Int) (cells : List (Int × Tyu.Conformance.Interval)) :
    Tyu.Conformance.MemModel.inRam (Tyu.Conformance.MemModel.bundle lo hi cells) a = true
      ↔ lo ≤ a ∧ a ≤ hi := by
  simp [Tyu.Conformance.MemModel.inRam, decide_eq_true_eq]

/-! ## T-D consolidation equivalence (P12 cleanup, item 1)

The conformance runner's operational bundle model (`MemModel.bundle`) and
the theorem-surface `BundleMem` are two mirrors of the same abstract
memory. Both aperture reads go through the single `wordDomain`; these
lemmas pin the two mirrors' observable loads to be equal (value-for-value
over the shared coercion) so neither can drift — the boundary-byte class of
defect (finding #1) is what these exist to catch. Registry entries:
`Tyu.Sound.TD.bundle_aperture_agrees`, `Tyu.Sound.TD.bundle_load_agrees`. -/

/-- Equivalence, aperture leg: the operational conformance read and the
denotational `BundleMem.apertureRead` agree (both through `wordDomain`). -/
theorem aperture_read_agrees (lo hi : Int)
    (cells : List (Int × Tyu.Conformance.Interval)) (w : Nat) :
    Tyu.Conformance.MemModel.apertureRead (Tyu.Conformance.MemModel.bundle lo hi cells) w
      = toConformance (BundleMem.apertureRead w) := by
  unfold Tyu.Conformance.MemModel.apertureRead BundleMem.apertureRead
  cases hp : Tyu.Conformance.wordDomain w with
  | none => simp [toConformance, hp]
  | some p => simp [toConformance, hp]

/-- Equivalence, load leg: a point-address load of the recorded cells reads
identically under both mirrors (the in-window case flows the recorded cell
through; the out-of-window case reads `top` on both sides). -/
theorem load_agrees (lo hi : Int) (cells : List (Int × Tyu.Conformance.Interval)) (a : Int)
    (w : Nat) :
    Tyu.Conformance.MemModel.load (Tyu.Conformance.MemModel.bundle lo hi cells)
        (Tyu.Conformance.Interval.range a a) w
      = toConformance (BundleMem.loadVal (ofCells lo hi cells) (IntervalVal.range a a)) := by
  have hload : BundleMem.loadVal (ofCells lo hi cells) (IntervalVal.range a a) =
      if (ofCells lo hi cells).inRam a then (ofCells lo hi cells).cell a else IntervalVal.top := by
    simp [BundleMem.loadVal]
  have hcell : (ofCells lo hi cells).cell a =
      fromConformance ((Tyu.Conformance.MemModel.bundle lo hi cells).recordedAt a) := by
    simp [ofCells]
  by_cases h1 : lo ≤ a <;> by_cases h2 : a ≤ hi
  · -- in-window: both mirrors flow the recorded cell through.
    have hin : (ofCells lo hi cells).inRam a := (ofCells_inRam_iff lo hi cells a).2 ⟨h1, h2⟩
    have hwinb : Tyu.Conformance.MemModel.inRam (Tyu.Conformance.MemModel.bundle lo hi cells) a = true := by
      simpa [Tyu.Conformance.MemModel.inRam, decide_eq_true_eq] using (And.intro h1 h2)
    rw [hload, if_pos hin, hcell, to_from_conformance, Tyu.Conformance.MemModel.load]
    simp [hwinb]
  · -- below the window: both mirrors read top.
    have hout : ¬ (ofCells lo hi cells).inRam a := by
      intro h
      exact h2 ((ofCells_inRam_iff lo hi cells a).1 h).2
    rw [hload, if_neg hout]
    rw [Tyu.Conformance.MemModel.load]
    simp [Tyu.Conformance.MemModel.inRam, toConformance, h1, h2, decide_eq_false, decide_eq_true_eq]
  · -- above the window: both mirrors read top.
    have hout : ¬ (ofCells lo hi cells).inRam a := by
      intro h
      exact h1 ((ofCells_inRam_iff lo hi cells a).1 h).1
    rw [hload, if_neg hout]
    rw [Tyu.Conformance.MemModel.load]
    simp [Tyu.Conformance.MemModel.inRam, toConformance, h1, h2, decide_eq_false, decide_eq_true_eq]
  · -- neither: both mirrors read top.
    have hout : ¬ (ofCells lo hi cells).inRam a := by
      intro h
      exact h1 ((ofCells_inRam_iff lo hi cells a).1 h).1
    rw [hload, if_neg hout]
    rw [Tyu.Conformance.MemModel.load]
    simp [Tyu.Conformance.MemModel.inRam, toConformance, h1, h2, decide_eq_false, decide_eq_true_eq]

/-- The bundle model as the generated `Tyu.Mem.MemModel` interface. The
interface is the statement-relativism surface (§Q3) — its `store`/write
return values are degenerate projections; the real T-D laws are proved over
`BundleMem` itself (like T-C is proved over `Tyu.Step.ConcreteMem`, not the
interface). -/
def ofBundle (m : BundleMem) : MemModel := {
  load := fun addr _w => m.loadVal addr,
  store := fun _addr v => v,
  apertureRead := fun _place w => BundleMem.apertureRead w,
  apertureWrite := fun _place v => v }

end Tyu.Bundles