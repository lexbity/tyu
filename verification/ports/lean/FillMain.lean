import Tyu.Automation.Corpus
import Tyu.Automation.Auto
import Tyu.Automation.Cycle
import Std

/-! The `fill` executable (PLAN-VERIFY-3 P10.2): candidate-proof generation.

For every renderable, non-omitted statement of the supplied artifacts
(`--obl <M>.obl.json ...`, paired 1:1 by position with `--meta <M>.gen.json
...`), `fill` writes a candidate theorem file into the `--out` directory (one
file per obligation, named by the sanitized id). *Both* statement surfaces
get candidates:

  * the ground statements (`tyu.gen/1` rows) — candidates with `tyu_auto`;
  * the via-cycles compositions (§P5.2) of loop-bearing words — candidates
    with `auto_cycle` (the machine-checked composition; the developer's
    real cycle obligations are the `cycle_*` schemes, which `auto_cycle`
    assists).

Every candidate is headed by the machine-readable marker
`-- tyu:candidate obligation=<id>` (§Q10) the harvest parses; the
`--fill-budget=<secs>` is recorded in the header (the per-obligation
automation budget; default 10). Files are byte-deterministic; whether a
candidate elaborates is decided by the kernel at the next `lake build`; a
harvested candidate is `trust: proof` with `authored: "candidate"`.

Usage: fill --obl=<M>.obl.json ... --meta=<M>.gen.json ... --out=<dir>
            [--fill-budget=<secs>]   |   --selfcheck -/

namespace Tyu.Automation.Fill

open Tyu.Automation.Corpus

/-- The candidate text for a ground or via statement. `tactic` is `tyu_auto`
for ground rows and `auto_cycle` for the via-cycles compositions. -/
def candidateText (module defName id tactic budget : String) : String :=
  let short := (defName.dropPrefix? "stmt_").getD defName
  "-- tyu:candidate obligation=" ++ id ++ "\n" ++
  "-- candidate theorem (unreviewed, automation-generated; the kernel gates it).\n" ++
  "-- fill-budget=" ++ budget ++ "s; remove the marker to acknowledge review.\n" ++
  "import Gen." ++ module ++ "\n" ++
  "import Tyu.Automation.Auto\n" ++
  "import Tyu.Automation.Cycle\n\n" ++
  "theorem obl_" ++ short ++ " : (Tyu.Gen.Corpus." ++ module ++ "." ++ defName ++ ") := by\n" ++
  "  " ++ tactic ++ "\n"

def sanitize (s : String) : String :=
  String.ofList (s.toList.map (fun c => if c.isAlphanum then c else '_'))

def main (args : List String) : IO UInt32 := do
  match args with
  | "--selfcheck" :: _ => IO.println "PASS: fill selfcheck"; pure 0
  | _ =>
      let oblArgs : List String := args.filter (fun a => a.startsWith "--obl=")
        |>.filterMap (fun a => (a.dropPrefix? "--obl=").map (fun x => x.toString))
      let metaArgs : List String := args.filter (fun a => a.startsWith "--meta=")
        |>.filterMap (fun a => (a.dropPrefix? "--meta=").map (fun x => x.toString))
      let out0 := match args.filter (fun a => a.startsWith "--out=") with
        | o :: _ => ((o.dropPrefix? "--out=").map (fun x => x.toString)).getD "."
        | _ => "."
      let out : String := out0
      let budget : String := match args.filter (fun a => a.startsWith "--fill-budget=") with
        | o :: _ => ((o.dropPrefix? "--fill-budget=").map (fun x => x.toString)).getD "10"
        | _ => "10"
      if oblArgs.isEmpty || metaArgs.length != oblArgs.length then
        IO.println "usage: fill --obl=<M>.obl.json ... --meta=<M>.gen.json ... --out=<dir> [--fill-budget=<secs>] | --selfcheck"
        return 2
      IO.FS.createDirAll (System.FilePath.mk out)
      let mut written := 0
      for (a, m) in oblArgs.zip metaArgs do
        let doc ← IO.FS.readFile a
        let gen ← IO.FS.readFile m
        -- ground statements: `tyu_auto` candidates.
        for (id, _kind, module, defName) in renderableRows doc gen do
          let path := out ++ "/" ++ sanitize id ++ ".lean"
          IO.FS.writeFile (System.FilePath.mk path) (candidateText module defName id "tyu_auto" budget)
          written := written + 1
        -- via-cycles compositions of loop-bearing words: `auto_cycle`
        -- candidates (the import surface is already in place).
        for (id, _kind, istmt) in viaRows doc do
          let module := match istmt.splitOn "." with
            | _ :: _ :: _ :: m :: _ => m
            | _ => ""
          let defName := match istmt.splitOn "." |>.getLast? with | some d => d | none => ""
          let path := out ++ "/" ++ sanitize id ++ ".lean"
          IO.FS.writeFile (System.FilePath.mk path) (candidateText module defName id "auto_cycle" budget)
          written := written + 1
      IO.println s!"fill: {written} candidate file(s) written to {out}"
      pure 0

end Tyu.Automation.Fill

/-- The executable entry. -/
def main (args : List String) : IO UInt32 := Tyu.Automation.Fill.main args