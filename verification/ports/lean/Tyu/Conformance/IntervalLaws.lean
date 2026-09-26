import Tyu.Conformance.Interval

/- In-build lattice law pins (kernel-checked `by decide` examples): the
   abstract interval domain must reproduce `verifier::interval` exactly
   (PLAN-VERIFY-3 P3.2 conformance). -/

namespace Tyu.Conformance

-- lattice identities (from verifier::interval tests)
example : Interval.join (Interval.bottom) (Interval.range 1 2) = Interval.range 1 2 := by decide
example : Interval.join (Interval.range 1 3) (Interval.range 7 9) = Interval.range 1 9 := by decide
example : Interval.subsetOf (Interval.range 1 2) (Interval.range 0 3) = true := by decide
example : Interval.add (Interval.range 1 3) (Interval.range 4 5) = Interval.range 5 8 := by decide
example : Interval.add (Interval.range 9223372036854775806 9223372036854775807) (Interval.range 1 1) = Interval.top := by decide
example : Interval.sub (Interval.range (-9223372036854775807) (-9223372036854775808)) (Interval.range 2 4) = Interval.top := by decide
example : Interval.mul (Interval.range (-2) 3) (Interval.range (-3) 4) = Interval.range (-9) 12 := by decide
example : Interval.castNarrow (Interval.range 150 150) 0 100 = Interval.bottom := by decide
example : evalInRange (Interval.range 50 50) 0 100 = Tri.defTrue := by decide
example : evalInRange (Interval.range 150 150) 0 100 = Tri.defFalse := by decide
example : evalInRange (Interval.range 50 200) 0 100 = Tri.top := by decide
example : evalInRange Interval.bottom 0 100 = Tri.top := by decide
example : triCmp (Interval.range 1 2) (Interval.range 5 7) "cmp_lt" = Tri.defTrue := by decide
example : triCmp Interval.top (Interval.range 1 2) "cmp_lt" = Tri.top := by decide
example : triCmp (Interval.range 5 5) (Interval.range 5 5) "cmp_eq" = Tri.defTrue := by decide
example : wordDomain 64 = none := by decide
example : wordDomain 32 = some (-2147483648, 2147483647) := by decide
example : Tyu.Conformance.toText (Interval.range 5 8) = "[5,8]" := by decide

end Tyu.Conformance
