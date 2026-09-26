import Tyu.Gen.Render
import Tyu.Gen.Sha256

/-! The `gen` executable (PLAN-VERIFY-3 P5).

Usage: gen --render --obl <artifact-file...> --out <dir>   |   gen --selfcheck

Fail-closed: an artifact that fails to render, or any unknown op mnemonic,
exits nonzero. -/

def renderErrMsg : Tyu.Gen.Render.RenderErr → String
  | Tyu.Gen.Render.RenderErr.notOblDoc m => m
  | Tyu.Gen.Render.RenderErr.unknownForm m => m

def renderOne (ob : String) (outDir : String) : IO String := do
  let doc ← IO.FS.readFile ob
  match Tyu.Gen.Render.renderModule doc with
  | Except.error e => pure ("FAIL: " ++ ob ++ ": " ++ renderErrMsg e)
  | Except.ok (leanText, metaText) =>
      let module := match Tyu.Gen.Render.parseArtifact doc with
        | Except.ok a => Tyu.Gen.Render.ident a.module
        | Except.error _ => "?"
      IO.FS.writeFile (outDir ++ "/" ++ module ++ ".lean") leanText
      IO.FS.writeFile (outDir ++ "/" ++ module ++ ".gen.json") metaText
      pure ("OK: " ++ ob ++ " → " ++ module ++ ".lean (+" ++ toString leanText.length ++ ")")

/-- Split `args` at the `--out` marker: files before, out-dir after. -/
def splitAtOut (args : List String) : List String × List String :=
  let rec go : List String → List String → List String × List String
    | [], acc => (acc.reverse, [])
    | "--out" :: rest, acc => (acc.reverse, rest)
    | a :: rest, acc => go rest (a :: acc)
  go args []

def renderMain (files : List String) (out : String) : IO UInt32 := do
  IO.FS.createDirAll out
  let mut failures := 0
  for f in files do
    let r ← renderOne f out
    IO.println r
    if r.startsWith "FAIL" then failures := failures + 1
  if failures == 0 then return 0 else return 1

def vecCheck : Bool :=
  (Tyu.Gen.Render.selfcheck).all (fun (s, want) => Tyu.Gen.Sha256.sha256Hex s == want)

def selfcheckMain : IO UInt32 := do
  if !vecCheck then
    IO.println "FAIL: SHA-256 known-answer vector mismatch"
    return 1
  IO.println "PASS: SHA-256 known-answer vectors"
  return 0

def main (args : List String) : IO UInt32 := do
  match args with
  | "--selfcheck" :: _ => selfcheckMain
  | "--render" :: rest =>
      let (files0, out) := splitAtOut rest
      let files := files0.filter (fun a => a ≠ "--obl")
      match out with
      | o :: _ => renderMain files o
      | [] =>
          IO.println "usage: gen --render --obl <files...> --out <dir>   |   gen --selfcheck"
          return 2
  | _ =>
      IO.println "usage: gen --render --obl <files...> --out <dir>   |   gen --selfcheck"
      return 2