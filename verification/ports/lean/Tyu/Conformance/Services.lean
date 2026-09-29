import Tyu.Services
import Tyu.Conformance.Json
import Tyu.Conformance.Parse

/-! The service-vector corpus runner (PLAN-VERIFY-3 P15.2).

The hosted bundle's `evidence/vectors.json` carries a `tyu.svcvec/1` corpus
of SCRIPTED channel programs: `[make, send, recv, …]` op lists with the
expected FIFO outputs. The port (`conformance --level services`) replays each
script against the abstract-atomic services model; the tooling suite's Rust
mirror (`verifier::services`) replays the SAME corpus, and the hosted
execution-test leg compiles+runs the same scripts on the hosted runtime —
three surfaces, one FIFO semantics, zero divergence. A step that reorders or
merges channel values diverges here, exactly as `tyu.vec/1` pins the IR
transfer (R9 detection). -/

namespace Tyu.ServicesConformance

open Tyu.Services

/-- One typed script of the `tyu.svcvec/1` corpus. -/
structure Script where
  id : String
  ops : List Tyu.Services.ServiceOp
  expect : List Int
  deriving Inhabited

/-- The scripted document. -/
structure SvcVecFile where
  schema : String
  triple : String
  scripts : List Script
  deriving Inhabited

/-- Parse one `["name", args…]` op. -/
def parseOp (j : Tyu.Conformance.Json) : Option Tyu.Services.ServiceOp :=
  match Tyu.Conformance.Json.asArr j with
  | none => none
  | some arr =>
      match ((arr.head?).bind Tyu.Conformance.Json.asStr) with
      | some "make" =>
          (arr.drop 1 |>.head?).bind Tyu.Conformance.Json.asInt
            |>.map (fun c => .make (Int.toNat c))
      | some "send" =>
          match (arr.drop 1 |>.head?).bind Tyu.Conformance.Json.asInt,
                (arr.drop 2 |>.head?).bind Tyu.Conformance.Json.asInt with
          | some c, some v => some (.send (Int.toNat c) v)
          | _, _ => none
      | some "recv" =>
          (arr.drop 1 |>.head?).bind Tyu.Conformance.Json.asInt
            |>.map (fun c => .recv (Int.toNat c))
      | _ => none

/-- Parse + schema-check a `tyu.svcvec/1` document. -/
def parseSvcVecFile (input : String) : Option SvcVecFile :=
  match Tyu.Conformance.parseJson input with
  | none => none
  | some j =>
      let schema := (Tyu.Conformance.Json.field j "schema").bind Tyu.Conformance.Json.asStr
      let triple := (Tyu.Conformance.Json.field j "triple").bind Tyu.Conformance.Json.asStr
      let raw := (Tyu.Conformance.Json.field j "scripts").bind Tyu.Conformance.Json.asArr |>.getD []
      let parsed := raw.filterMap (fun s =>
        match (Tyu.Conformance.Json.field s "id").bind Tyu.Conformance.Json.asStr with
        | some id =>
            let ops := (Tyu.Conformance.Json.field s "ops").bind Tyu.Conformance.Json.asArr
                |>.getD [] |>.filterMap parseOp
            let expect := (Tyu.Conformance.Json.field s "expect").bind Tyu.Conformance.Json.asArr
                |>.getD [] |>.filterMap Tyu.Conformance.Json.asInt
            some ({ id := id, ops := ops, expect := expect } : Script)
        | none => none)
      match schema, triple with
      | some s, some t =>
          if s ≠ "tyu.svcvec/1" then none
          else if parsed.length ≠ raw.length then none
          else some ({ schema := s, triple := t, scripts := parsed } : SvcVecFile)
      | _, _ => none

/-- The mismatches of one script against its expectations. -/
def checkScript (sc : Script) : Option String :=
  let outputs : Option (List Int) :=
    match Tyu.Services.traceRun sc.ops (default) with
    | some (stack, _) => some stack
    | none => none
  match outputs with
  | some stack =>
      if stack == sc.expect then none
      else some ("got=" ++ String.intercalate "," (stack.map toString) ++
                 " want=" ++ String.intercalate "," (sc.expect.map toString))
  | none => some "blocked (a recv on a non-make-local channel)"

/-- Replay a whole corpus: the mismatching script ids + count. -/
def runFile (f : SvcVecFile) : List String :=
  f.scripts.filterMap (fun sc =>
    match checkScript sc with
    | some m => some ("script '" ++ sc.id ++ "': " ++ m)
    | none => none)

end Tyu.ServicesConformance