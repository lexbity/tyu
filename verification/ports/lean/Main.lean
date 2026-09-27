import Tyu.Conformance.Runner
import Tyu.Conformance.Fragment
import Tyu.Conformance.IntervalLaws
import Tyu.Stackmeta
import Tyu.IR.Semantics

open Tyu.Conformance
open Tyu.Stackmeta

/-- The conformance replay (P3.2): every `tyu.vec/1` vector reproduced
byte-exactly by the abstract transfer. -/
def conformanceMain (args : List String) : IO UInt32 := do
  -- `--` and `--corpus` are separators, never corpus dirs: `lake exe
  -- conformance -- --corpus <dir>` passes the `--` through, and treating it
  -- as a path turned a usage form into an uncaught IO exception (P3 audit).
  let corpusDirs := args.filter fun a => a ≠ "--corpus" && a ≠ "--"
  if corpusDirs.isEmpty then
    IO.println "usage: conformance [--corpus <dir> ...]"
    return 1
  let mut total := 0
  let mut failures := 0
  for dir in corpusDirs do
    let path := dir ++ "/index.json"
    -- A missing/unreadable corpus is a counted FAIL, never an uncaught
    -- exception: the gate must fail closed through the RESULT line.
    let content ←
      try
        IO.FS.readFile path
      catch _ =>
        pure ""
    if content == "" then
      IO.println s!"FAIL: {path}: unreadable or empty (not a tyu.vec/1 corpus)"
      failures := failures + 1
      continue
    match parseVecFile content with
    | none =>
        IO.println s!"FAIL: {path}: not a valid tyu.vec/1 document (schema/malformed)"
        failures := failures + 1
    | some f =>
        let mismatches := runVecFile f
        total := total + f.vectors.length
        if mismatches.isEmpty then
          IO.println s!"PASS: {path} ({f.triple}): {f.vectors.length} vectors, zero divergence"
        else
          for m in mismatches do
            IO.println s!"DIVERGE: {f.triple} [{m.id}]: got head={m.gotHead} top={m.gotTop}; want head={m.wantHead} top={m.wantTop}"
          failures := failures + mismatches.length
  IO.println s!"RESULT: vectors={total} mismatches={failures}"
  if failures == 0 then return 0 else return 1

/-- The stackmeta replay (P4.2's T-C empirical hook): re-derive each corpus
word's (net, high) from its blocks text + call-sig map via the proved
monoid; net must match exactly, the peak envelope must stay within the
declared bound (the sound direction). -/
def stackmetaMain (jsons : List String) : IO UInt32 := do
  if jsons.isEmpty then
    IO.println "FAIL: no *.json stackmeta goldens supplied (pass them after --stackmeta)"
    return 1
  let mut totalWords := 0
  let mut skipped := 0
  let mut failures := 0
  for path in jsons do
    let content ←
      try
        IO.FS.readFile path
      catch _ =>
        pure ""
    if content == "" then
      IO.println s!"FAIL: {path}: unreadable or empty (not a tyu.stackmeta/1 document)"
      failures := failures + 1
      continue
    match parseMetaFile content with
    | none =>
        IO.println s!"FAIL: {path}: not a valid tyu.stackmeta/1 document"
        failures := failures + 1
    | some f =>
        let skips := (f.words.filter (fun w => w.unresolved)).length
        let bads := checkFile f
        totalWords := totalWords + f.words.length
        skipped := skipped + skips
        if bads.isEmpty then
          IO.println s!"PASS: {path} ({f.module}): {f.words.length} words re-derived (net exact, peak within declared; {skips} unresolved-skip)"
        else
          for b in bads do
            IO.println s!"STACKMETA-DIVERGE: {f.module} [{b.name}]: status={b.status} net={b.net}/{b.gotNet} high={b.high}/{b.gotHigh}"
          failures := failures + bads.length
  IO.println s!"RESULT: stackmeta words={totalWords} skipped={skipped} divergences={failures}"
  if failures == 0 then return 0 else return 1

def main (args : List String) : IO UInt32 := do
  match args with
  | "--level" :: "fragment" :: rest =>
      -- The shared `tyu.fragvec/1` corpus (P9.3): the fragment surface's
      -- mechanical Lean↔Rust pin. Reads `<dir>/index.json` per corpus dir.
      let dirs := rest.filter fun a => a ≠ "--corpus" && a ≠ "--"
      if dirs.isEmpty then
        IO.println "usage: conformance --level fragment --corpus <dir>..."
        return 1
      let mut total := 0
      let mut failures := 0
      for dir in dirs do
        let path := dir ++ "/index.json"
        let content ←
          try
            IO.FS.readFile path
          catch _ =>
            pure ""
        if content == "" then
          IO.println s!"FAIL: {path}: unreadable or empty (not a tyu.fragvec/1 corpus)"
          failures := failures + 1
          continue
        match parseFragFile content with
        | none =>
            IO.println s!"FAIL: {path}: not a valid tyu.fragvec/1 document (schema/malformed)"
            failures := failures + 1
        | some f =>
            let mismatches := runFragFile f
            total := total + f.programs.length
            if mismatches.isEmpty then
              IO.println s!"PASS: {path} ({f.triple}): {f.programs.length} fragment programs, zero divergence"
            else
              for m in mismatches do
                IO.println s!"FRAG-DIVERGE: {f.triple} [{m.id}]: got {m.got}; want {m.want}"
              failures := failures + mismatches.length
      IO.println s!"RESULT: fragment={total} mismatches={failures}"
      if failures == 0 then return 0 else return 1
  | "--level" :: "stackmeta" :: rest =>
      let files := rest.filter (fun a => a ≠ "--stackmeta")
      if files.isEmpty then
        IO.println "usage: conformance --level stackmeta --stackmeta <golden-files...>"
        return 1
      stackmetaMain files
  | _ =>
      conformanceMain (args.filter fun a => a ≠ "--corpus")
