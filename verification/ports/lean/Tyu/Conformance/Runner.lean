import Tyu.Conformance.Json

namespace Tyu.Conformance

/-- Execute one vector against the abstract engine, mirroring
`verifier::tests::vector_corpus::Vector::run`. -/
def runVector (v : Vector) (wordBits : Nat) : Tri × Interval :=
  let sr := percentRange
  let mem := match v.model with | "aperture" => MemModel.apertureEmpty | _ => MemModel.flat
  match v.mode with
  | "cfg" =>
      let blocks := parseBlocks v.blocks
      let cf := runCfg blocks v.sigIn v.sigOut sr wordBits mem
      let exit := exitState blocks cf sr wordBits mem
      let val := exit.topInterval
      (evalInRange val v.targetLo v.targetHi, val)
  | _ =>
      let entry := parseIntervals v.entry
      let ops := parseOps v.ops
      let st0 : State := { stack := entry.map Slot.computed, locals := (State.fresh 64).locals }
      let (stF, pre) := ops.foldl (fun (acc : State × Option Interval) o =>
          let (st, pre) := acc
          let pre' := if o.form == Tyu.IR.OpForm.cast && o.subtypeCast then some st.topInterval else pre
          (stepOp o st sr wordBits mem, pre'))
        (st0, none)
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
    let (head, topIv) := runVector v f.wordBits
    let gotH := Tri.name head
    let gotT := Tyu.Conformance.toText topIv
    if gotH ≠ v.expectHead || gotT ≠ v.expectTop then
      some { id := v.id, gotHead := gotH, wantHead := v.expectHead, gotTop := gotT, wantTop := v.expectTop }
    else none)

end Tyu.Conformance
