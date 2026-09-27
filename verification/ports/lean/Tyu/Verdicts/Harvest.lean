import Tyu.Gen.Render
import Tyu.Gen.Sha256
import Tyu.Conformance.Json
import Std
import Lean.Elab.Command

/-! The harvest (PLAN-VERIFY-3 P7.1): kernel-checked developer theorems become
`tyu.verdicts/v2` verdicts.

The harvest runs INSIDE the generated package — its environment IS the
developer's proofs (the package's `Harvest.lean` is `import proofs` + this
module, run as a `#eval!`d `CommandElabM` action via `lake env lean
<Harvest.lean>`, the interpreter vehicle `AxiomAudit.lean` already uses). For
PORT fixtures the environment is the corpus golden statements instead.

Per module it:
  1. reads the artifact (`tyu.obl/v2`) and its Gen metadata (`tyu.gen/1`);
  2. verifies every expected `stmt_*` def EXISTS in the kernel environment
     by its deterministic name (`Tyu.Gen.Corpus.<M>.<stmtName>` from the
     module/word/kind/occurrence the artifact declares) — missing ⇒ E6420;
  3. finds the matching `obl_*` theorem and requires its TYPE to be
     definitionally equal to the statement def's type (the type-level
     binding — a proof of the wrong statement cannot be harvested, Q4);
  4. axiom-audits the theorem (`Lean.collectAxioms`): only the benign set
     `{propext, Quot.sound, Classical.choice}` is permitted; the placeholder
     axiom and `Lean.ofReduceBool` and any other axiom are E6419 (fail-closed);
  5. emits the deterministic `tyu.verdicts/v2` document (ids sorted, fixed
     key order — byte-identical across runs).

A statement with no theorem is `open` (unproven is a state, not a fault).
Inputs arrive as environment variables (the plan's gen-dir/obl/out CLI
surface, delivered through the process environment — the interpreter vehicle
cannot take positional argv); exit codes propagate from `IO.Process.exit`. -/

namespace Tyu.Verdicts.Harvest

open Lean Elab Command
open Tyu.Conformance

-- --------------------------------------------------------------------------
-- Inputs (environment vars)
-- --------------------------------------------------------------------------

structure Inputs where
  genDir : String
  obl : String
  out : String

def getInputs : CommandElabM (Option Inputs) := do
  let g ← liftIO $ IO.getEnv "TYU_HARVEST_GEN_DIR"
  let o ← liftIO $ IO.getEnv "TYU_HARVEST_OBL"
  let out ← liftIO $ IO.getEnv "TYU_HARVEST_OUT"
  match g, o, out with
  | some g, some o, some out =>
      if g == "" || o == "" || out == "" then pure none
      else pure (some ⟨g, o, out⟩)
  | _, _, _ => pure none

-- --------------------------------------------------------------------------
-- Gen metadata rows (`tyu.gen/1`)
-- --------------------------------------------------------------------------

structure StmtMeta where
  id : String
  omitted : Bool
  reason : String
  defName : String
  statementHash : String
  srcDef : String
  deriving Inhabited

structure GenMeta where
  module : String
  statements : List StmtMeta

partial def parseStmtMeta (j : Conformance.Json) : Option StmtMeta := do
  let id ← (j.field "id") >>= Json.asStr
  let omitted ← (j.field "omitted") >>= Json.asBool
  let reason := ((j.field "reason") >>= Json.asStr).getD ""
  let defName := ((j.field "def") >>= Json.asStr).getD ""
  let statementHash := ((j.field "statement_hash") >>= Json.asStr).getD ""
  let srcDef := ((j.field "src_def") >>= Json.asStr).getD ""
  pure ⟨id, omitted, reason, defName, statementHash, srcDef⟩

def parseGenMeta (input : String) : Option GenMeta :=
  match parseJson input with
  | none => none
  | some j =>
      match (j.field "schema") >>= Json.asStr with
      | some "tyu.gen/1" =>
          let module := ((j.field "module") >>= Json.asStr).getD ""
          let rows := ((j.field "statements") >>= Json.asArr).getD []
          let parsed := rows.filterMap parseStmtMeta
          if parsed.length ≠ rows.length then none
          else some ⟨module, parsed⟩
      | _ => none

-- --------------------------------------------------------------------------
-- Kernel-environment surface
-- --------------------------------------------------------------------------

/-- The renderer's error as a String (RenderErr derives Repr, not ToString). -/
def renderErrString : Tyu.Gen.Render.RenderErr → String
  | .notOblDoc m => m
  | .unknownForm m => m

def envConstants : CommandElabM (List Name) := do
  let env ← getEnv
  pure <| env.constants.fold (fun (acc : List Name) name _ => name :: acc) []

/-- The full name of a statement def: `Tyu.Gen.Corpus.<Module>.<defName>` — a
hierarchical name built component-wise (`Name.mkSimple` keeps the dots in one
component, which never matches the environment's names). -/
def dottedName (s : String) : Name :=
  s.splitOn "." |>.foldl (fun acc c => if c == "" then acc else Name.str acc c) Name.anonymous

def fullStmtName (module defName : String) : Name :=
  dottedName ("Tyu.Gen.Corpus." ++ module ++ "." ++ defName)

/-- Environment contains (stem-independent exact-name check). -/
def envHas (n : Name) : CommandElabM Bool := do
  let env ← getEnv
  pure (env.contains n)

/-- A constant's dotted string name. -/
def nameString (n : Name) : String := n.toString

-- --------------------------------------------------------------------------
-- Statement binding, theorem search, axiom audit
-- --------------------------------------------------------------------------

structure TheoremHit where
  name : String
  axioms : List String
  deriving Inhabited

/-- The benign axiom set (§Q11 item 2). -/
def permittedAxioms : List Name :=
  [ ``propext, ``Quot.sound, ``Classical.choice ]

/-- Find the `obl_*` theorem(s) of a statement by the type-level binding: a
constant whose (dotted) name ends with `obl_<stmtName-without-stmt_>` and
whose type is defeq to the statement DEF itself (a theorem of `stmt_X` has
type `stmt_X`). The canonical name (Q4) is preferred; any type-matching
`obl_*` is accepted. -/
def findTheorem (stmtName : Name) (defName : String) :
    CommandElabM (Array TheoremHit) := do
  let short := (defName.dropPrefix? "stmt_").getD defName
  let expectedSuffix := "obl_" ++ short
  let stmtExpr := Lean.mkConst stmtName
  let targets ← envConstants
  let mut exact : Array TheoremHit := #[]
  let mut any : Array TheoremHit := #[]
  for c in targets do
    let cstr := nameString c
    if ¬ cstr.endsWith expectedSuffix then continue
    let thmType ← liftTermElabM <| Lean.Meta.inferType (Lean.mkConst c)
    let ok ← liftTermElabM <| Lean.Meta.isDefEq thmType stmtExpr
    if ok then
      let axioms := (← Lean.collectAxioms c).toList.map (fun a => nameString a)
      let hit := ⟨cstr, axioms⟩
      exact := exact.push hit
  if exact.isEmpty then pure any else pure exact

/-- The axiom audit (§Q11 item 2): only the benign set is permitted. -/
def auditAxioms (axioms : List String) : Bool :=
  axioms.all fun a => permittedAxioms.any fun p => p.toString == a

-- --------------------------------------------------------------------------
-- Deterministic JSON output (hand-rolled; reuse the renderer's escapes)
-- --------------------------------------------------------------------------

def jobjOf (kvs : List (String × String)) : String :=
  "{" ++ String.intercalate "," (kvs.map (fun (k, v) => Tyu.Gen.Render.jstr k ++ ":" ++ v)) ++ "}"
def jarrOf (xs : List String) : String :=
  "[" ++ String.intercalate "," xs ++ "]"

/-- A discharged certificate record (§6.3). `surface` is the proof surface
(`"ir"` or `"source"`); `relies` names the registry theorems the surface
rests on (`["T-S"]` for a source certificate, §Q2 — emitted only when
nonempty). -/
def certRecord (surface : String) (relies : List String) (id : String) (idHash : String)
    (statementHash : String) (thmName : String) (file : String) : String :=
  let proofKvs : List (String × String) :=
    [ ("kind", Tyu.Gen.Render.jstr "certificate")
    , ("statement", Tyu.Gen.Render.jstr "tyu.stmt/1.0")
    , ("theorem", Tyu.Gen.Render.jstr thmName)
    , ("kernel_check", Tyu.Gen.Render.jstr "lean-kernel+lean4checker")
    , ("file", Tyu.Gen.Render.jstr file) ] ++
    match relies with
    | [] => []
    | rs => [ ("relies", jarrOf (rs.map (fun r => Tyu.Gen.Render.jstr r))) ]
  jobjOf
    [ ("id", Tyu.Gen.Render.jstr id)
    , ("id_hash", Tyu.Gen.Render.jstr idHash)
    , ("status", Tyu.Gen.Render.jstr "discharged")
    , ("trust", Tyu.Gen.Render.jstr "proof")
    , ("method", Tyu.Gen.Render.jstr "certificate")
    , ("surface", Tyu.Gen.Render.jstr surface)
    , ("statement_hash", Tyu.Gen.Render.jstr statementHash)
    , ("authored", Tyu.Gen.Render.jstr "developer")
    , ("proof", jobjOf proofKvs) ]

/-- An open record (unproven is a state, not a fault). -/
def openRecord (id : String) (idHash : String) (witness : String) : String :=
  jobjOf
    [ ("id", Tyu.Gen.Render.jstr id), ("id_hash", Tyu.Gen.Render.jstr idHash)
    , ("status", Tyu.Gen.Render.jstr "open"), ("trust", Tyu.Gen.Render.jstr "open")
    , ("witness", jobjOf [("reason", Tyu.Gen.Render.jstr witness)]) ]

-- --------------------------------------------------------------------------
-- The per-module harvest
-- --------------------------------------------------------------------------

/-- Insertion sort by id (small, deterministic). -/
partial def sortBy (f : String → String) : List String → List String
  | [] => []
  | x :: xs =>
      let smaller := sortBy f (xs.filter (fun y => f y < f x))
      let larger  := sortBy f (xs.filter (fun y => f x ≤ f y))
      smaller ++ x :: larger

/-- Harvest one module: env-binding + theorems + axiom audit, then the records
sorted by obligation id. `Except.error msg` is a hard harvest failure (E6419
class — axiom/tamper) that propagates as a nonzero exit. The second list is
the axiom-audit evidence rows (theorem, sorted axioms) for
`tyu.axiom-audit/1`. -/
def harvestModule (inputs : Inputs) (document : String) :
    CommandElabM (Except String (String × String × String × List String ×
                                 List (String × List String))) := do
  let a ← match Tyu.Gen.Render.parseArtifact document with
    | .error e => return .error ("artifact parse: " ++ renderErrString e)
    | .ok a => pure a
  let genText ← liftIO $ IO.FS.readFile (inputs.genDir ++ "/" ++ a.module ++ ".gen.json")
  let gm ← match parseGenMeta genText with
    | some m => pure m
    | none => return .error ("gen metadata unparseable for " ++ a.module)
  if gm.module ≠ a.module then
    return .error ("gen metadata module mismatch: " ++ gm.module ++ " ≠ " ++ a.module)
  let mut records : List (String × String) := []   -- (id, json)
  let mut auditRows : List (String × List String) := []   -- (theorem, axioms)
  for o in a.obligations do
    let rowMeta := gm.statements.find? (fun m => m.id == o.id)
    match rowMeta with
    | none =>
        records := (o.id, openRecord o.id o.idHash "no-gen-row") :: records
    | some m =>
        if m.omitted then
          records := (o.id, openRecord o.id o.idHash (if m.reason == "" then "omitted" else m.reason)) :: records
        else
          let full := fullStmtName a.module m.defName
          let has ← envHas full
          if ¬ has then
            return .error ("E6420: statement def missing from environment: " ++ full.toString)
          let hitIr ← findTheorem full m.defName
          match hitIr[0]? with
          | some thm =>
              if ¬ auditAxioms thm.axioms then
                return .error ("E6419: axiom audit failed for " ++ thm.name
                  ++ " — axioms: " ++ toString thm.axioms)
              auditRows := (thm.name, sortBy (fun s => s) thm.axioms) :: auditRows
              records := (o.id, certRecord "ir" [] o.id o.idHash m.statementHash thm.name
                           ("proofs/" ++ a.module ++ ".lean")) :: records
          | none =>
              -- P9: source-surface certificates — a theorem of `src_stmt_…`
              -- (typed at the generated source-surface declaration) is bound
              -- with surface "source" and relies ["T-S"] (§Q2: the source
              -- claim discharges the IR obligation in composition with T-S).
              if m.srcDef == "" then
                records := (o.id, openRecord o.id o.idHash "unproven") :: records
              else
                let srcFull := fullStmtName a.module m.srcDef
                let hasSrc ← envHas srcFull
                if ¬ hasSrc then
                  return .error ("E6420: source stmt def missing from environment: " ++ srcFull.toString)
                let hitSrc ← findTheorem srcFull m.defName
                match hitSrc[0]? with
                | some thm =>
                    if ¬ auditAxioms thm.axioms then
                      return .error ("E6419: axiom audit failed for " ++ thm.name
                        ++ " — axioms: " ++ toString thm.axioms)
                    auditRows := (thm.name, sortBy (fun s => s) thm.axioms) :: auditRows
                    records := (o.id, certRecord "source" ["T-S"] o.id o.idHash m.statementHash thm.name
                                 ("proofs/" ++ a.module ++ ".lean")) :: records
                | none =>
                    records := (o.id, openRecord o.id o.idHash "unproven") :: records
  -- deterministic: sort by the obligation id (the record keys)
  let ids := records.map Prod.fst
  let sorted := sortBy (fun s => s) ids
  let byId := fun id => (records.filter (fun p => Prod.fst p == id))
  let final := sorted.flatMap (fun id => byId id |>.map Prod.snd)
  pure (.ok (a.module, a.target, a.modelSemantics, final, auditRows))

-- --------------------------------------------------------------------------
-- The `#eval!` runner
-- --------------------------------------------------------------------------

/-- Insertion sort by the theorem name (deterministic audit emission). -/
partial def sortByFst : List (String × List String) → List (String × List String)
  | [] => []
  | x :: xs =>
      let smaller := sortByFst (xs.filter (fun y => y.1 < x.1))
      let larger := sortByFst (xs.filter (fun y => x.1 ≤ y.1))
      smaller ++ x :: larger

/-- The per-module axiom-audit evidence (`tyu.axiom-audit/1`): every
harvested theorem with its transitive axiom set, next to the permitted set
it was checked against (§Q11 item 2 — the `evidence/axiom_audit.json`
shape, per module; tyu re-homes it to
`.tyu-verify/harvest/<Module>.axiom_audit.json`). -/
def auditDoc (module target model : String)
    (rows : List (String × List String)) : String :=
  let thms := jarrOf ((sortByFst rows).map (fun (name, axs) =>
    jobjOf [ ("axioms", jarrOf (axs.map (fun a => Tyu.Gen.Render.jstr a)))
           , ("theorem", Tyu.Gen.Render.jstr name) ]))
  jobjOf
    [ ("module", Tyu.Gen.Render.jstr module)
    , ("model_semantics", Tyu.Gen.Render.jstr model)
    , ("permitted_axioms", jarrOf (permittedAxioms.map (fun p => Tyu.Gen.Render.jstr p.toString)))
    , ("schema", Tyu.Gen.Render.jstr "tyu.axiom-audit/1")
    , ("status", Tyu.Gen.Render.jstr "ok")
    , ("target", Tyu.Gen.Render.jstr target)
    , ("theorems", thms) ]

/-- The file-level certifier identity (§6.3, §Q11). -/
def certifierJson : String :=
  jobjOf
    [ ("class", Tyu.Gen.Render.jstr "port"), ("name", Tyu.Gen.Render.jstr "lean")
    , ("recognition", Tyu.Gen.Render.jstr "tyu-port/lean/1")
    , ("tool", jobjOf [("name", Tyu.Gen.Render.jstr "harvest"), ("version", Tyu.Gen.Render.jstr "0.1.0")])
    , ("toolchain", Tyu.Gen.Render.jstr "lean4:4.27.0") ]

/-- The full `tyu.verdicts/v2` document. -/
def makeDocument (target model : String) (records : List String) : String :=
  jobjOf
    [ ("schema", Tyu.Gen.Render.jstr "tyu.verdicts/v2")
    , ("certifier", certifierJson)
    , ("semantics", Tyu.Gen.Render.jstr "tyu.ir-sem/1.0")
    , ("stmt", Tyu.Gen.Render.jstr "tyu.stmt/1.0")
    , ("target", Tyu.Gen.Render.jstr target)
    , ("model_semantics", Tyu.Gen.Render.jstr model)
    , ("verdicts", jarrOf records) ]

unsafe def run : CommandElabM Unit := do
  let some inputs ← getInputs
    | liftIO (IO.eprintln
        "usage: TYU_HARVEST_GEN_DIR=<gen> TYU_HARVEST_OBL=<obl> TYU_HARVEST_OUT=<out> lake env lean <Harvest.lean>")
        *> liftIO (IO.Process.exit 2)
  let oblText ← liftIO $ IO.FS.readFile inputs.obl
  match ← harvestModule inputs oblText with
  | .error err =>
      -- The `#eval!` vehicle loses stderr; write the diagnosable reason into
      -- the out file so the caller (tyu) can surface E6419/E6420/… context.
      liftIO $ IO.FS.writeFile inputs.out
        ("{\"schema\":\"tyu.harvest-error/1\",\"message\":" ++ Tyu.Gen.Render.jstr err ++ "}")
      liftIO (IO.eprintln ("harvest: " ++ err))
      liftIO (IO.Process.exit 1)
  | .ok (module, target, model, records, auditRows) =>
      let doc := makeDocument target model records
      liftIO $ IO.FS.writeFile inputs.out doc
      liftIO $ IO.FS.writeFile (inputs.out ++ ".audit.json")
        (auditDoc module target model auditRows)

end Tyu.Verdicts.Harvest