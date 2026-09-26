import Tyu.Conformance.VectorRun

namespace Tyu.Conformance

/-- A minimal hand-rolled JSON reader (P3.2: `tyu.vec/1` is consumed by
own code; schema-version checked at the caller). Whitespace-skipping
recursive descent over `List Char`; `partial` is confined to this parser
(runtime, not theorem material). -/

def skipWs : List Char → List Char := fun cs => cs.dropWhile (fun c => c == ' ' || c == '\t' || c == '\n' || c == '\r')

partial def parseStringTail (acc : List Char) : List Char → Option (String × List Char)
  | [] => none
  | '"' :: r => some (String.ofList acc.reverse, r)
  | '\\' :: c :: r =>
      match c with
      | '"' => parseStringTail ('"' :: acc) r
      | '\\' => parseStringTail ('\\' :: acc) r
      | '/' => parseStringTail ('/' :: acc) r
      | 'b' => parseStringTail (Char.ofNat 8 :: acc) r
      | 'f' => parseStringTail (Char.ofNat 12 :: acc) r
      | 'n' => parseStringTail (Char.ofNat 10 :: acc) r
      | 'r' => parseStringTail (Char.ofNat 13 :: acc) r
      | 't' => parseStringTail (Char.ofNat 9 :: acc) r
      | _ => none
  | c :: r => parseStringTail (c :: acc) r


partial def parseNumber (rest : List Char) : Option (Json × List Char) :=
  let cs := skipWs rest
  let allowed : Char → Bool := fun c => ('0' ≤ c && c ≤ '9') || c == '-'
  let (digits, r) := cs.span allowed
  if digits.isEmpty then none
  else
    match parseInt (String.ofList digits) with
    | some n => some (Json.jnum n, r)
    | none => none

mutual
partial def parseObjectItems (acc : List (String × Json)) : List Char → Option (Json × List Char)
  | cs =>
      let r1 := skipWs cs
      match r1 with
      | '}' :: r => some (Json.jobj acc.reverse, r)
      | _ =>
          match r1 with
          | '"' :: rk =>
              match parseStringTail [] rk with
              | some (k, r2) =>
                  let r3 := skipWs r2
                  match r3 with
                  | ':' :: r4 =>
                      match parseValue r4 with
                      | none => none
                      | some (v, r5) =>
                          let r6 := skipWs r5
                          match r6 with
                          | ',' :: r7 => parseObjectItems ((k, v) :: acc) r7
                          | '}' :: r7 => some (Json.jobj ((k, v) :: acc).reverse, r7)
                          | _ => none
                  | _ => none
              | none => none
          | _ => none

partial def parseArrayItems (acc : List Json) : List Char → Option (Json × List Char)
  | cs =>
      let r1 := skipWs cs
      match r1 with
      | ']' :: r => some (Json.jarr acc.reverse, r)
      | _ =>
          match parseValue r1 with
          | none => none
          | some (v, r2) =>
              let r3 := skipWs r2
              match r3 with
              | ',' :: r4 => parseArrayItems (v :: acc) r4
              | ']' :: r4 => some (Json.jarr (v :: acc).reverse, r4)
              | _ => none

partial def parseValue (rest : List Char) : Option (Json × List Char) :=
  let cs := skipWs rest
  match cs with
  | [] => none
  | '{' :: r => parseObjectItems [] r
  | '[' :: r => parseArrayItems [] r
  | '"' :: r => (parseStringTail [] r).map (fun (str, rr) => (Json.jstr str, rr))
  | 't' :: r => if r.take 3 == "rue".toList then some (Json.jbool true, r.drop 3) else none
  | 'f' :: r => if r.take 4 == "alse".toList then some (Json.jbool false, r.drop 4) else none
  | 'n' :: r => if r.take 3 == "ull".toList then some (Json.jnull, r.drop 3) else none
  | _ => parseNumber cs
end

/-- Parse a whole document — everything after the root value must be
whitespace (trailing garbage fails). -/
def parseJson (input : String) : Option Json :=
  match parseValue input.toList with
  | some (j, rest) =>
      let rest' := skipWs rest
      if rest'.isEmpty then some j else none
  | none => none

/-- The typed `tyu.vec/1` document. -/
structure VecFile where
  schema : String
  triple : String
  slotBytes : Nat
  wordBits : Nat
  archTag : Nat
  vectors : List Vector
  deriving DecidableEq, Repr, Inhabited

/-- Parse + schema-check a `tyu.vec/1` document. -/
def parseVecFile (input : String) : Option VecFile :=
  match parseJson input with
  | none => none
  | some j =>
      let schema := (j.field "schema").bind Json.asStr
      let triple := (j.field "triple").bind Json.asStr
      let target := j.field "target"
      let sb := target.bind (fun t => (t.field "slot_bytes").bind Json.asInt)
      let wb := target.bind (fun t => (t.field "word_bits").bind Json.asInt)
      let archTag := target.bind (fun t => (t.field "arch_tag").bind Json.asInt)
      match schema, triple, sb, wb, archTag with
      | some s, some t, some sl, some w, some ar =>
          if s ≠ "tyu.vec/1" then none
          else
            let vs := (j.field "vectors").bind Json.asArr |>.getD []
            let parsed := vs.filterMap vectorFromJson
            if parsed.length ≠ vs.length then none
            else some (VecFile.mk s t sl.toNat w.toNat ar.toNat parsed)
      | _, _, _, _, _ => none

end Tyu.Conformance