import Tyu.Abs
import Tyu.Conformance.Interval

/-! The abstract transfer (`stepOp`/`absRun`) IS `Tyu.Abs` (PLAN-VERIFY-3
P14.1, P4-audit consolidation — the port carries exactly ONE interval
implementation): this file re-exports the abstract state and transfer into
`Tyu.Conformance` so the conformance runner keeps its public names while
every definition below lives once, in `Tyu/Abs.lean`, under
`namespace Tyu.Abs`. -/

namespace Tyu.Conformance

export Tyu.Abs (Slot)
export Tyu.Abs (State)
export Tyu.Abs (MemModel)
export Tyu.Abs (MemModel.flat MemModel.apertureEmpty MemModel.bundle)
export Tyu.Abs (OpInst)

namespace Slot

abbrev top := Tyu.Abs.Slot.top
abbrev computed := Tyu.Abs.Slot.computed

end Slot

namespace State

abbrev fresh := Tyu.Abs.State.fresh
abbrev calleeEntry := Tyu.Abs.State.calleeEntry
abbrev push' := Tyu.Abs.State.push'
abbrev popVal' := Tyu.Abs.State.popVal'
abbrev pop2 := Tyu.Abs.State.pop2
abbrev binop := Tyu.Abs.State.binop
abbrev topInterval := Tyu.Abs.State.topInterval
abbrev truncateStack := Tyu.Abs.State.truncateStack

end State

namespace MemModel

abbrev inRam := Tyu.Abs.MemModel.inRam
abbrev cellsOf := Tyu.Abs.MemModel.cellsOf
abbrev recordedAt := Tyu.Abs.MemModel.recordedAt
abbrev storeCells := Tyu.Abs.MemModel.storeCells
abbrev load := Tyu.Abs.MemModel.load
abbrev store := Tyu.Abs.MemModel.store
abbrev scriptedRead := Tyu.Abs.MemModel.scriptedRead
abbrev apertureRead := Tyu.Abs.MemModel.apertureRead

end MemModel

namespace OpInst

abbrev opMk := Tyu.Abs.OpInst.opMk
abbrev setConst := Tyu.Abs.OpInst.setConst
abbrev setConstBool := Tyu.Abs.OpInst.setConstBool
abbrev setSlot := Tyu.Abs.OpInst.setSlot
abbrev setSubtypeCast := Tyu.Abs.OpInst.setSubtypeCast
abbrev setBrTgt := Tyu.Abs.OpInst.setBrTgt
abbrev setBrIf := Tyu.Abs.OpInst.setBrIf
abbrev setPlace := Tyu.Abs.OpInst.setPlace

end OpInst

abbrev MAX_SLOTS := Tyu.Abs.MAX_SLOTS
abbrev percentRange := Tyu.Abs.percentRange
abbrev stepOp := Tyu.Abs.stepOp
abbrev absRun := Tyu.Abs.absRun

end Tyu.Conformance