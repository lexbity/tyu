import Tyu.Conformance.Json

namespace Tyu.Conformance

/-- Execute one vector against the abstract engine, mirroring
`verifier::tests::vector_corpus::Vector::run`. `ram` is the corpus's
optional bundle RAM window (P12.2); the `"bundle"` model runs an
`ApertureMem` instance over it, `"aperture"` runs the empty version. The
vector's scripted `reads` (place-keyed fixed intervals, P13.2) seed the
bundle instance's refined-register reads — applied on the Rust side too, so
the Rust↔Lean agreement covers a refined READ VALUE. -/
def runVector (v : Vector) (wordBits : Nat) (ram : Option (Int × Int)) : Tri × Interval :=
  let sr := percentRange
  let scripted := v.scripted.map (fun (p, lo, hi) => (p, Interval.range lo hi))
  let mem0 : MemModel := match v.model, ram with
    | "bundle", some (lo, hi) => MemModel.bundle lo hi [] scripted
    | "bundle", none => MemModel.apertureEmpty
    | "aperture", _ => MemModel.apertureEmpty
    | _, _ => MemModel.flat
  match v.mode with
  | "cfg" =>
      let blocks := parseBlocks v.blocks
      let (mem1, cf) := runCfg blocks v.sigIn v.sigOut sr wordBits mem0
      let exit := exitState blocks cf sr wordBits mem1
      let val := exit.topInterval
      (evalInRange val v.targetLo v.targetHi, val)
  | _ =>
      let entry := parseIntervals v.entry
      let ops := parseOps v.ops
      let st0 : State := { stack := entry.map Slot.computed, locals := (State.fresh 64).locals }
      let (_, (stF, pre)) := ops.foldl (fun (acc : MemModel × (State × Option Interval)) o =>
          let (mem', (st, pre)) := acc
          let pre' := if o.form == Tyu.IR.OpForm.cast && o.subtypeCast then some st.topInterval else pre
          let (mem'', st') := stepOp o st sr wordBits mem'
          (mem'', (st', pre')))
        (mem0, (st0, none))
      let val := if v.castSite then pre.getD Interval.top else stF.topInterval
      (evalInRange val v.targetLo v.targetHi, val)

/-- One observed mismatch. -/
structure Mismatch where
  id : String
  gotHead : String
  wantHead : String
  gotTop : String
  wantTop : String
  deriving DecidableEq, Repr

/-- Run a whole vector file; returns the mismatches. -/
def runVecFile (f : VecFile) : List Mismatch :=
  f.vectors.filterMap (fun v =>
    let (head, topIv) := runVector v f.wordBits f.ram
    let gotH := Tri.name head
    let gotT := Tyu.Conformance.toText topIv
    if gotH ≠ v.expectHead || gotT ≠ v.expectTop then
      some { id := v.id, gotHead := gotH, wantHead := v.expectHead, gotTop := gotT, wantTop := v.expectTop }
    else none)

end Tyu.Conformance
