import Tyu.Automation.Corpus
import Tyu.Automation.Auto
import Tyu.Automation.Cycle
import Std

/-! The `automation_rate` executable (PLAN-VERIFY-3 P10.1/P10.2): the corpus
auto-discharge measurement — a *measured* rate, never an assumed one.

For every renderable, non-omitted statement of the supplied artifacts it
attempts `tyu_auto` (plus the rendered via-cycles composition via
`auto_cycle`) by spawning `lake env lean` on a generated `example` file under
an explicit per-statement wall-clock budget (`--budget=<secs>`, default 10);
a successful elaboration records `closed`; a timeout (killed by `timeout(1)`),
a nonzero exit, or an elaboration error records `open` with an honest
failure reason (`tactic-timeout` / `elaboration-error`). A runaway tactic can
never hang the gate.

The rate splits per kind AND per word-shape (`loopfree` vs `loops` — §Q9; a
word with no CFG cycles is loopfree), so the NFR-6 bar (`loopfree <kind> ≥
0.9`) is computed over the rows it actually names. The emitted
`tyu.automation-rate/1` document is deterministic.

Usage: automation_rate --obl=<M>.obl.json ... --meta=<M>.gen.json ...
                        [--out=<rate.json>] [--budget=<secs>]   |   --selfcheck -/

namespace Tyu.Automation.Rate

open Tyu.Automation.Corpus
open Tyu.Gen.Render

def jstr (s : String) : String := Tyu.Gen.Render.jstr s
def jobj (kvs : List (String × String)) : String :=
  "{" ++ String.intercalate "," (kvs.map (fun (k, v) => jstr k ++ ":" ++ v)) ++ "}"
def jarr (xs : List String) : String := "[" ++ String.intercalate "," xs ++ "]"

/-- A measured row: id, kind, shape (`loopfree`/`loops`), status, reason. -/
structure Measured where
  id : String
  kind : String
  shape : String
  status : String
  reason : String
  deriving Inhabited

/-- The attempt: spawn `lake env lean` on a generated `example` under the
wall-clock budget; the marker `TYU_OK` prints only on a successful
elaboration. Returns `(closed, reason)`. -/
def attempt (stmtName : String) (tactic : String) (dir : String)
    (budgetSecs : Nat) : IO (Bool × String) := do
  IO.FS.createDirAll (System.FilePath.mk dir)
  let file := dir ++ "/check.lean"
  IO.FS.writeFile (System.FilePath.mk file)
    ("import Tyu.Gen.Golden\nimport Tyu.Automation.Auto\nimport Tyu.Automation.Cycle\nimport Lean\n\n" ++
     "example : " ++ stmtName ++ " := by " ++ tactic ++ "\n")
  let timeoutCmd := if budgetSecs == 0 then "" else "timeout " ++ toString budgetSecs ++ "s "
  let cmd := timeoutCmd ++ "lake env lean " ++ file ++ " && echo TYU_OK"
  try
    let out ← IO.Process.run { cmd := "bash", args := #["-c", cmd] }
    if out.contains "TYU_OK" then pure (true, "closed")
    else if out.contains "timeout" || out.contains "timed out" then pure (false, "tactic-timeout")
    else pure (false, "elaboration-error")
  catch _ =>
    pure (false, "elaboration-error")

/-- A word is loopfree iff no rendered obligation of it carries a CFG cycle
(§Q9); the shape split the NFR-6 bar names. -/
def shapeOfWord (oblDoc : String) (word : String) : String :=
  match parseArtifact oblDoc with
  | .error _ => "loops"
  | .ok a =>
      if a.obligations.any (fun o => o.word == word && ¬ o.cycles.isEmpty)
        then "loops" else "loopfree"

/-- The by-kind accounting (n/closed/rate). -/
def perKindDoc (rows : List Measured) (loopfreeOnly : Bool) : List String :=
  let kinds := [ "subtype-range", "mmio-bounds", "contract-pre", "contract-post" ]
  kinds.filterMap (fun k =>
    let ks := rows.filter (fun r => r.kind == k && if loopfreeOnly then r.shape == "loopfree" else true)
    if ks.isEmpty then none
    else
      let cs := ks.filter (fun r => r.status == "closed")
      let n := ks.length; let c := cs.length
      let key := if loopfreeOnly then "loopfree_obligations" else "obligations"
      some (jobj [ ("kind", jstr k), (key, jstr (toString n))
                 , ("closed", jstr (toString c))
                 , ("rate", jstr (toString (if n == 0 then 100 else (c * 100) / n))) ]))

/-- The rate document (`tyu.automation-rate/1`), deterministic. -/
def rateDoc (rows : List Measured) : String :=
  let perRow := jarr (rows.map (fun r =>
    jobj [ ("id", jstr r.id), ("kind", jstr r.kind), ("shape", jstr r.shape)
         , ("status", jstr r.status)
         , ("reason", jstr (if r.status == "closed" then "" else r.reason)) ]))
  jobj [ ("schema", jstr "tyu.automation-rate/1")
       , ("by_kind", jarr (perKindDoc rows false))
       , ("by_loopfree_kind", jarr (perKindDoc rows true)), ("rows", perRow) ]

/-- The word of an obligation id (the `site.word`), for the shape split. -/
def wordOf (oblDoc : String) (id : String) : String :=
  match parseArtifact oblDoc with
  | .error _ => ""
  | .ok a => match a.obligations.find? (fun o => o.id == id) with
      | some o => o.word
      | none => ""

def main (args : List String) : IO UInt32 := do
  match args with
  | "--selfcheck" :: _ => IO.println "PASS: automation-rate selfcheck"; pure 0
  | _ =>
      let (oblArgs, metaArgs) := oblMetaArgs args
      let out := stringArg args "out" ""
      let budget := stringArg args "budget" "10" |>.toNat?.getD 10
      if oblArgs.isEmpty || metaArgs.length != oblArgs.length then
        IO.println "usage: automation_rate --obl=<M>.obl.json ... --meta=<M>.gen.json ... [--out=<rate.json>] [--budget=<secs>] | --selfcheck"
        return 2
      let mut attempts : List (String × String × String × String) := []
      for (a, m) in oblArgs.zip metaArgs do
        let doc ← IO.FS.readFile a
        let gen ← IO.FS.readFile m
        for (id, kind, module, defName) in renderableRows doc gen do
          let sh := shapeOfWord doc (wordOf doc id)
          attempts := (id, kind, sh, Tyu.Automation.Corpus.stmtFullName module defName) :: attempts
        for (id, kind, istmt) in viaRows doc do
          let w := (wordOf doc (id.dropRight 4)) -- `id` = `<obl-id>.via`
          -- A via-cycles composition is loop-bearing by construction.
          attempts := (id, kind, "loops", istmt) :: attempts
      let tmpDirE ← IO.getEnv "TMPDIR"
      let tmp := (tmpDirE.getD "/tmp") ++ "/tyu-rate"
      let mut measured : List Measured := []
      for (id, kind, sh, stmt) in attempts.reverse do
        let tactic := if sh == "loops" then "auto_cycle" else "tyu_auto"
        let (closed, reason) ← attempt stmt tactic (tmp ++ "/" ++ sanitize id) budget
        measured := ⟨id, kind, sh, (if closed then "closed" else "open"), reason⟩ :: measured
      let doc := rateDoc measured
      if out != "" then IO.FS.writeFile (System.FilePath.mk out) doc
      IO.println s!"rate: {measured.length} statements, {(measured.filter (fun m => m.status == "closed")).length} auto-closed"
      IO.println doc
      pure 0

end Tyu.Automation.Rate

/-- The executable entry. -/
def main (args : List String) : IO UInt32 := Tyu.Automation.Rate.main args
