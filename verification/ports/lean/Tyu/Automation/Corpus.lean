import Tyu.Gen.Render
import Tyu.Conformance.Json

/-! Shared corpus plumbing for the automation executables (PLAN-VERIFY-3
P10): parse `tyu.obl/v2` artifacts and `tyu.gen/1` metadata into the
statement rows the rate/fill mains drive. Deliberately no dependency on the
harvest's elaboration module — these run as plain executables. -/

namespace Tyu.Automation.Corpus

open Tyu.Gen.Render

/-- One metadata row: the rendered statement's id, def name, omission. -/
structure Row where
  id : String
  defName : String
  omitted : Bool
  deriving Inhabited

/-- Parse a `tyu.gen/1` document into its statement rows (hand-rolled over
`Tyu.Conformance.Json`; unknown keys skipped). -/
def parseGenMetaRows (input : String) : Option (String × List Row) :=
  match Tyu.Conformance.parseJson input with
  | none => none
  | some j =>
      match (j.field "schema") >>= Tyu.Conformance.Json.asStr with
      | some "tyu.gen/1" =>
          let module := ((j.field "module") >>= Tyu.Conformance.Json.asStr).getD ""
          let rows := ((j.field "statements") >>= Tyu.Conformance.Json.asArr).getD []
          let parsed := rows.filterMap (fun r =>
            let id := (r.field "id") >>= Tyu.Conformance.Json.asStr
            let defName := (r.field "def") >>= Tyu.Conformance.Json.asStr
            let omitted := ((r.field "omitted") >>= Tyu.Conformance.Json.asBool).getD false
            match id with
            | some id => some ⟨id, defName.getD "", omitted⟩
            | none => none)
          if parsed.length == rows.length then some (module, parsed) else none
      | _ => none

/-- The obligation kind from its id: `<Module>::<word>::<kind>::<occ>`. -/
def kindOfId (id : String) : String :=
  match id.splitOn "::" with
  | _ :: _ :: k :: _ => k
  | _ => "unknown"

/-- The full generated statement name for a `(module, defName)` pair. -/
def stmtFullName (module defName : String) : String :=
  "Tyu.Gen.Corpus." ++ module ++ "." ++ defName

/-- The renderable (non-omitted) statement rows of an artifact+meta pair:
`(id, kind, module, defName)`. -/
def renderableRows (oblDoc genDoc : String) : List (String × String × String × String) :=
  match parseArtifact oblDoc with
  | .error _ => []
  | .ok a =>
      match parseGenMetaRows genDoc with
      | none => []
      | some (_, rows) =>
          rows.filterMap (fun r =>
            if r.omitted then none
            else some (r.id, kindOfId r.id, a.module, r.defName))

/-- The via-cycles composition rows for loop-bearing out-range obligations:
`(id + ".via", kind, <stmt>_via_cycles name)`. -/
def viaRows (oblDoc : String) : List (String × String × String) :=
  match parseArtifact oblDoc with
  | .error _ => []
  | .ok a =>
      a.obligations.filterMap (fun o =>
        match Tyu.Gen.Render.classify a o, Tyu.Gen.Render.oelRoot o.oel with
        | none, some ("out", _) =>
            if ! o.cycles.isEmpty then
              some (o.id ++ ".via", o.kind,
                ("Tyu.Gen.Corpus." ++ a.module ++ "." ++
                 Tyu.Gen.Render.viaName a.module o.word o.kind o.occurrence))
            else none
        | _, _ => none)

end Tyu.Automation.Corpus