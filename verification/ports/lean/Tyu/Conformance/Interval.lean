import Std

namespace Tyu.Conformance

/-- The abstract interval lattice: bottom (empty), a closed range on i64, top (whole i64 domain).
Mirrors `verifier::interval::Interval`. -/
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

end Tyu.Conformance


