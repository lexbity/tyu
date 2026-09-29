import Tyu.Abs.Rederive

/-! The `rederive` executable (PLAN-VERIFY-3 P14.2, §Q12).

Usage:
    rederive --selfcheck
    rederive --corpus <dir...>            (the differential pin: re-run
                                           `tyu.vec/1` program corpora)
    rederive --obl <artifact...> --out <file> [--toolchain <pin>]

Fail-closed: an artifact that fails to parse, a corpus mismatch, or any
unknown form exits nonzero. The `--obl` output is a `tyu.verdicts/v2`
document: `trust: proof`, `method: rederive`, `proof.kind: rederive`,
statement-bound — the automation layer's `proven`-admissible upgrade. -/

/-- Split `args` at the first `marker`: (before, after). -/
def splitAt (marker : String) (args : List String) : List String × List String :=
  let rec go : List String → List String → List String × List String
    | [], acc => (acc.reverse, [])
    | m :: rest, acc => if m == marker then (acc.reverse, rest) else go rest (m :: acc)
  go args []

def corpusMain (dirs : List String) : IO UInt32 := do
  if dirs.isEmpty then
    IO.println "usage: rederive --corpus <dir...>"
    return 2
  let mut total := 0
  let mut failures := 0
  for dir in dirs do
    let content ←
      try
        IO.FS.readFile (dir ++ "/index.json")
      catch _ =>
        pure ""
    if content == "" then
      IO.println s!"FAIL: {dir}/index.json unreadable (not a rederive corpus)"
      failures := failures + 1
      continue
    match Tyu.Rederive.runCorpus content with
    | Except.error e =>
        IO.println s!"FAIL: {dir}/index.json: {e}"
        failures := failures + 1
    | Except.ok (n, mismatches) =>
        total := total + n
        if mismatches.isEmpty then
          IO.println s!"PASS: {dir}/index.json: {n} programs re-derived, zero divergence"
        else
          for m in mismatches do
            IO.println s!"DIVERGE: [{m.id}]: got {m.gotHead}/{m.gotTop}; want {m.wantHead}/{m.wantTop}"
          failures := failures + mismatches.length
  IO.println s!"RESULT: rederive programs={total} divergences={failures}"
  if failures == 0 then return 0 else return 1

def oblMain (files : List String) (out : Option String) (toolchain : String) : IO UInt32 := do
  if files.isEmpty then
    IO.println "usage: rederive --obl <artifact...> --out <verdicts.json> [--toolchain <pin>]"
    return 2
  let mut failures := 0
  for f in files do
    let doc ←
      try
        IO.FS.readFile f
      catch _ =>
        pure ""
    if doc == "" then
      IO.println s!"FAIL: {f} unreadable"
      failures := failures + 1
      continue
    match Tyu.Rederive.rederiveArtifact doc toolchain with
    | Except.error e =>
        IO.println s!"FAIL: {f}: {e}"
        failures := failures + 1
    | Except.ok verdicts =>
        match out with
        | none => IO.println verdicts
        | some path => do
            IO.FS.writeFile path verdicts
            IO.println s!"OK: {f} → {path}"
  IO.println s!"RESULT: rederive modules={files.length - failures} failures={failures}"
  if failures == 0 then return 0 else return 1

def selfcheckMain : IO UInt32 := do
  -- SHA-256 self-check (the statement binding relies on the port SHA-256).
  let ok := (Tyu.Gen.Render.selfcheck).all (fun (s, want) => Tyu.Gen.Sha256.sha256Hex s == want)
  if !ok then
    IO.println "FAIL: SHA-256 known-answer vector mismatch"
    return 1
  IO.println "PASS: SHA-256 known-answer vectors"
  return 0

def main (args : List String) : IO UInt32 := do
  match args with
  | "--selfcheck" :: _ => selfcheckMain
  | "--corpus" :: rest =>
      let dirs := rest.filter (fun a => a ≠ "--" && a ≠ "--corpus")
      corpusMain dirs
  | "--obl" :: rest =>
      -- rest: <files...> [--toolchain <pin>] [--out <file>]
      let (preOut, outPart) := splitAt "--out" rest
      let (files, tcPart) := splitAt "--toolchain" preOut
      let toolchain := match tcPart with | t :: _ => t | [] => "lean4:4.27.0+unpinned"
      oblMain (files.filter (fun a => a ≠ "--obl")) (outPart.head?) toolchain
  | _ =>
      IO.println "usage: rederive --selfcheck | --corpus <dir...> | --obl <artifacts...> --out <file> [--toolchain <pin>]"
      return 2