import Tyu.Conformance.Runner
import Tyu.Conformance.Fragment
import Tyu.Conformance.IntervalLaws
import Tyu.Conformance.Services
import Tyu.Stackmeta
import Tyu.IR.Semantics
import Tyu.Bundles

open Tyu.Conformance
open Tyu.Stackmeta
open Tyu.ServicesConformance

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
    -- P12.2: a corpus document is `<dir>/index.json` (the shared per-target
    -- corpora) or `<dir>/vectors.json` (the bundle evidence corpora); the
    -- first readable candidate wins.
    let mut content : String := ""
    let mut path : String := ""
    for candidate in [dir ++ "/index.json", dir ++ "/vectors.json"] do
      let c ←
        try
          IO.FS.readFile candidate
        catch _ =>
          pure ""
      if c ≠ "" && content == "" then
        content := c
        path := candidate
    if content == "" then
      IO.println s!"FAIL: {dir}/index.json or vectors.json: unreadable or empty (not a tyu.vec/1 corpus)"
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

/-- The bundle-geometry report (PLAN-VERIFY-3 P12.2): every modeled
bundle instance's RAM window, as JSON. The Rust
`bundle_instance_conformance` suite pins these numbers against the
bundles' `model/model.toml [memory] ram` artifacts — the single shared
source the Rust `ApertureMem` instances, this port's instances, and the
`evidence/` corpus headers all consume. -/
def geometryEntry (name : String) (b : Tyu.Bundles.BundleMem) : String :=
  "{\"id\":\"" ++ name ++ "\",\"ramLo\":" ++ toString b.ramLo ++ ",\"ramHi\":" ++ toString b.ramHi ++ "}"

def bundlesMain (_ : List String) : IO UInt32 := do
  let parts : List String :=
    [ geometryEntry "x86_64" Tyu.Bundles.x86_64,
      geometryEntry "armv7m" Tyu.Bundles.armv7m,
      geometryEntry "riscv32" Tyu.Bundles.riscv32,
      geometryEntry "rp2350" Tyu.Bundles.rp2350 ]
  let joined := parts.foldl (fun acc p => if acc == "" then p else acc ++ "," ++ p) ""
  IO.println ("{\"schema\":\"tyu.bind/1\",\"bundles\":[" ++ joined ++ "]}")
  return 0

/-- The refinement-band report (PLAN-VERIFY-3 P13.2, P3 finding): each
modeled bundle instance's declared refinement DEVICE values — the datasheet
band (`uartFrBand`) and the access-mode — as JSON. The Rust
`bundle_instance_conformance` suite pins these against the bundles'
`model/model.toml [[refinements.device]] mask`/`mode` declarations, so a
datasheet transcription is a reviewed artifact-pair (Lean instance +
manifest), never prose. The band the registry consumes
(`Tyu.Sound.TD.uartfr_band_domain_at_device`) equals this report's value
mechanically. -/
def bandsEntry (refinement : String) (mask : Int) (mode : String) : String :=
  "{\"id\":\"" ++ refinement ++ "\",\"mask\":" ++ toString mask ++ ",\"mode\":\"" ++ mode ++ "\"}"

def bandsMain (_ : List String) : IO UInt32 := do
  let parts : List String :=
    [ bandsEntry "rp2350.uart-fr" Tyu.Bundles.uartFrBand (Tyu.Bundles.AccessMode.s Tyu.Bundles.uartFrMode) ]
  let joined := parts.foldl (fun acc p => if acc == "" then p else acc ++ "," ++ p) ""
  IO.println ("{\"schema\":\"tyu.bands/1\",\"devices\":[" ++ joined ++ "]}")
  return 0

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
  | "--level" :: "services" :: rest =>
      -- The `tyu.svcvec/1` service corpus (P15.2): the abstract-atomic
      -- services model's scripted channel programs, replayed by the port
      -- (and — zero divergence — by the Rust mirror + the hosted runtime
      -- leg). Reads `<dir>/vectors.json` from each corpus dir.
      let dirs := rest.filter fun a => a ≠ "--corpus" && a ≠ "--"
      if dirs.isEmpty then
        IO.println "usage: conformance --level services --corpus <dir>..."
        return 1
      let mut total := 0
      let mut failures := 0
      for dir in dirs do
        let path := dir ++ "/vectors.json"
        let content ←
          try
            IO.FS.readFile path
          catch _ =>
            pure ""
        if content == "" then
          IO.println s!"FAIL: {path}: unreadable or empty (not a tyu.svcvec/1 corpus)"
          failures := failures + 1
          continue
        match parseSvcVecFile content with
        | none =>
            IO.println s!"FAIL: {path}: not a valid tyu.svcvec/1 document (schema/malformed)"
            failures := failures + 1
        | some f =>
            let mismatches := runFile f
            total := total + f.scripts.length
            if mismatches.isEmpty then
              IO.println s!"PASS: {path} ({f.triple}): {f.scripts.length} service scripts, zero divergence"
            else
              for m in mismatches do
                IO.println s!"SVC-DIVERGE: {f.triple} {m}"
              failures := failures + mismatches.length
      IO.println s!"RESULT: services={total} mismatches={failures}"
      if failures == 0 then return 0 else return 1
  | "--level" :: "bundles" :: _ =>
      -- P12.2: the bundle-instance geometry report (no corpus needed).
      bundlesMain []
  | "--level" :: "bands" :: _ =>
      -- P13.2: the refinement-band report (the datasheet mask/access-mode,
      -- pinned against the manifest by bundle_instance_conformance).
      bandsMain []
  | "--level" :: "stackmeta" :: rest =>
      let files := rest.filter (fun a => a ≠ "--stackmeta")
      if files.isEmpty then
        IO.println "usage: conformance --level stackmeta --stackmeta <golden-files...>"
        return 1
      stackmetaMain files
  | _ =>
      conformanceMain (args.filter fun a => a ≠ "--corpus")
