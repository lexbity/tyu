import Tyu.Abs

/-! THE single interval implementation is `Tyu.Abs` (PLAN-VERIFY-3 P14.1,
P4-audit consolidation): this file re-exports it into `Tyu.Conformance` so
the conformance runner, the bundle instances, the fragment corpus, and the
stackmeta path keep their public names while the port carries exactly ONE
interval implementation (two interval layers that can drift are the failure
mode this architecture exists to prevent). -/

namespace Tyu.Conformance

export Tyu.Abs (Interval)
export Tyu.Abs (Interval.bottom Interval.top Interval.range)
export Tyu.Abs (Tri)
export Tyu.Abs (Tri.defTrue Tri.defFalse Tri.top)

namespace Interval

abbrev isBottom := Tyu.Abs.Interval.isBottom
abbrev isTop := Tyu.Abs.Interval.isTop
abbrev join := Tyu.Abs.Interval.join
abbrev subsetOf := Tyu.Abs.Interval.subsetOf
abbrev i64Min := Tyu.Abs.Interval.i64Min
abbrev i64Max := Tyu.Abs.Interval.i64Max
abbrev inI64Domain := Tyu.Abs.Interval.inI64Domain
abbrev Contains := Tyu.Abs.Interval.Contains
abbrev add := Tyu.Abs.Interval.add
abbrev sub := Tyu.Abs.Interval.sub
abbrev mul := Tyu.Abs.Interval.mul
abbrev castNarrow := Tyu.Abs.Interval.castNarrow
abbrev widenOld := Tyu.Abs.Interval.widenOld

end Interval

namespace Tri

abbrev name := Tyu.Abs.Tri.name

end Tri

abbrev evalInRange := Tyu.Abs.evalInRange
abbrev triFromBoolIv := Tyu.Abs.triFromBoolIv
abbrev boolIv := Tyu.Abs.boolIv
abbrev triNot := Tyu.Abs.triNot
abbrev triAnd := Tyu.Abs.triAnd
abbrev triOr := Tyu.Abs.triOr
abbrev triCmp := Tyu.Abs.triCmp
abbrev wordDomain := Tyu.Abs.wordDomain
abbrev toText := Tyu.Abs.toText

end Tyu.Conformance