import Tyu.Conformance.Cfg

namespace Tyu.Conformance

/-- A minimal JSON value (hand-rolled reader per P3.2: `tyu.vec/1` is read
with own code, no library dependency). -/
inductive Json where
  | jnull
  | jbool (b : Bool)
  | jnum (v : Int)
  | jstr (s : String)
  | jarr (xs : List Json)
  | jobj (flds : List (String × Json))
  deriving Repr, Inhabited

namespace Json

def isStr : Json → Option String
  | .jstr s => some s
  | _ => none

def field (self : Json) (name : String) : Option Json :=
  match self with
  | .jobj flds => flds.lookup name
  | _ => none

def asBool : Json → Option Bool
  | .jbool b => some b
  | _ => none

def asInt : Json → Option Int
  | .jnum v => some v
  | _ => none

def asStr : Json → Option String
  | .jstr s => some s
  | _ => none

def asArr : Json → Option (List Json)
  | .jarr xs => some xs
  | _ => none

end Json

/-- Split a string on a character. -/
def splitChar (c : Char) (cs : List Char) : List (List Char) :=
  let (groups, cur) := cs.foldl
    (fun (acc : List (List Char) × List Char) ch =>
      if ch == c then (acc.2.reverse :: acc.1, []) else (acc.1, ch :: acc.2))
    ([], [])
  (cur.reverse :: groups).reverse

def splitOnStr (sep : Char) (s : String) : List String :=
  (splitChar sep s.toList).map String.ofList

def dropWhileChar (p : Char → Bool) (cs : List Char) : List Char :=
  cs.dropWhile p

def trim (s : String) : String :=
  String.ofList (dropWhileChar (fun c => c == ' ' || c == '\t' || c == '\n' || c == '\r') s.toList)

/-- Drop a prefix if present (4.27's `String.dropPrefix` returns a Slice;
this returns the remainder as a plain String). -/
def dropPrefixStr (pre : String) (s : String) : Option String :=
  let p := pre.toList
  let cs := s.toList
  if p.isPrefixOf cs then some (String.ofList (cs.drop p.length)) else none

/-- Parse a decimal integer (optionally negative). -/
def parseNatDigitsAux : List Char → Option Nat
  | [] => none
  | cs =>
      cs.foldl (fun (a : Option Nat) c =>
        if '0' ≤ c && c ≤ '9' then some (a.getD 0 * 10 + (c.toNat - '0'.toNat)) else none) (some 0)

def parseInt (s : String) : Option Int :=
  let cs := trim s |>.toList
  match cs with
  | [] => none
  | '-' :: rest => (parseNatDigitsAux rest).map (fun n => - (n : Int))
  | _ => (parseNatDigitsAux cs).map (fun n => (n : Int))

def parseNat (s : String) : Option Nat :=
  match trim s |>.toList with
  | [] => none
  | cs => cs.foldl (fun (a : Option Nat) c =>
      if '0' ≤ c && c ≤ '9' then some (a.getD 0 * 10 + (c.toNat - '0'.toNat)) else none) (some 0)

/-- Parse an interval token: `<top>`, `<bottom>`, `[lo,hi]`. -/
def parseInterval (s : String) : Option Interval :=
  let t := trim s
  if t == "<top>" then some Interval.top
  else if t == "<bottom>" then some Interval.bottom
  else
    let inner := (dropPrefixStr "[" t).bind (fun x => dropPrefixStr "" x)
      |>.bind (fun x => dropSuffixStr "]" x)
    match inner with
    | none => none
    | some body =>
        match splitOnStr ',' body with
        | [lo, hi] => match parseInt lo, parseInt hi with
            | some l, some h => if l ≤ h then some (Interval.range l h) else none
            | _, _ => none
        | _ => none
where
  dropSuffixStr (suf : String) (s : String) : Option String :=
    let p := suf.toList
    let cs := s.toList
    if p.isSuffixOf cs then some (String.ofList (cs.take (cs.length - p.length))) else none

/-- Parse the entry-stack text: `";"`-separated interval tokens. -/
def parseIntervals (s : String) : List Interval :=
  (splitOnStr ';' s).filterMap parseInterval

end Tyu.Conformance