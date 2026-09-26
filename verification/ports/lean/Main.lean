import Tyu.Conformance.Runner
import Tyu.Conformance.IntervalLaws
import Tyu.IR.Semantics

open Tyu.Conformance

def main (args : List String) : IO UInt32 := do
  if args.isEmpty then
    IO.println "usage: conformance [--corpus <dir> ...]"
    IO.println "  each dir must contain index.json (tyu.vec/1)"
    return 1
  let corpusDirs := args.filter fun a => a ≠ "--corpus"
  if corpusDirs.isEmpty then
    IO.println "error: no corpus directories given"
    return 1
  let mut total := 0
  let mut failures := 0
  for dir in corpusDirs do
    let path := dir ++ "/index.json"
    let content ← IO.FS.readFile path
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
