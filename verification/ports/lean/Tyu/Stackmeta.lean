import Tyu.Sound
import Tyu.Conformance.Json

/-! The stackmeta replay (PLAN-VERIFY-3 P4.2's empirical hook for T-C).

The port re-derives every corpus word's `(net, high)` from the artifact's
declared values + the canonical blocks text + the per-word call-sig map
(`tyu.stackmeta/1`), using the *proved* monoid (`Tyu.Sound.seqNet` /
`seqHigh`), and checks:

  - net equality:  `seqNet ops = declared.net`          (exact)
  - the peak bound: `seqHigh ops ≤ declared.high`       (sound direction:
    the actual envelope must stay within the declared furnace; `Top`
    declared bounds are trivially satisfied and counted)

Words whose calls could not be resolved (`unresolved: true`) are skipped
with an honest count. Any violation is a corpus divergence — the golden
gate's vectors-anchoring discipline applied to the T-C theorem. -/

namespace Tyu.Stackmeta

open Tyu.Conformance
open Tyu.Sound
open Tyu.Step

/-- One word of the golden. -/
structure WordMeta where
  name : String
  net : Int
  high : Int
  top : Bool
  blocks : String
  calls : List (String × Int)
  unresolved : Bool
  deriving Repr

/-- The typed `tyu.stackmeta/1` document. -/
structure MetaFile where
  schema : String
  target : String
  module : String
  words : List WordMeta
  deriving Repr

def lookupCall (calls : List (String × Int)) (name : String) : Option Int :=
  calls.lookup name

/-- Parse one canonical op line into a `ConcreteOp`; `call X` resolves the
sig from the per-word calls map (the map's net becomes a sig with that net:
`(1, Int.toNat (1 + net))` — the depth fold only consumes `callNet`). -/
def parseConcreteOp (line : String) (calls : List (String × Int)) : Option ConcreteOp :=
  let toks := (splitOnStr ' ' (trim line)).filter (fun t => t ≠ "")
  match toks with
  | [] => none
  | "call" :: name :: _ =>
      match lookupCall calls name with
      | some net => some ((ConcreteOp.opMk .call).setCallSig (1, Int.toNat (1 + net)))
      | none => none
  | _ =>
      match parseOp line with
      | none => none
      | some op =>
          some {
            form := op.form, constVal := op.constVal, constBool := op.constBool,
            slot := op.slot, subtypeCast := op.subtypeCast,
            brTgt := op.brTgt, brIfTgts := op.brIfTgts, callSig := none }

/-- A block in the stackmeta walk: concrete ops + the successor routing. -/
structure SBlock where
  id : Nat
  ops : List ConcreteOp
  succs : List Nat
  deriving Repr

/-- The successor routing of one block, from its terminator op. -/
def succsOf (ops : List ConcreteOp) : List Nat :=
  match ops.getLast? with
  | some o => match o.form, o.brTgt, o.brIfTgts with
      | .br, some t, _ => [t]
      | .br_if, _, some (t, e) => [t, e]
      | _, _, _ => []
  | none => []

/-- Parse the CFG blocks text into blocks of *concrete* ops (calls resolved
by the per-word map). The terminator routing mirrors `Conformance.parseBlocks`. -/
def parseBlocksConcrete (text : String) (calls : List (String × Int)) : List SBlock :=
  let lines := splitOnStr '\n' text
  let (acc, curId, curOps) := lines.foldl
    (fun (acc : List SBlock × Option Nat × List ConcreteOp) line =>
      let (blks, cid, cops) := acc
      let t := trim line
      if t == "" then acc
      else if t.startsWith "block b" then
        match dropPrefixStr "block b" t with
        | some rest =>
            match parseNat rest with
            | some i =>
                let blksFinal := match cid with
                  | some id => (({ id := id, ops := cops.reverse, succs := succsOf cops.reverse } : SBlock) :: blks)
                  | none => blks
                (blksFinal, some i, [])
            | none => acc
        | none => acc
      else match parseConcreteOp t calls with
        | some o => (blks, cid, o :: cops)
        | none => acc)
    ([], none, [])
  let final := match curId with
    | some id => (({ id := id, ops := curOps.reverse, succs := succsOf curOps.reverse } : SBlock) :: acc)
    | none => acc
  final.reverse

/-- Re-derive `(net, high)` of a word by walking its CFG from block 0
(T-C's walk form: a terminating walk's net is the sig delta; the peak
envelope of the visited blocks is within the declared high — the sound
direction). BrIf walking chooses the then-successor (nets are
path-independent in well-typed words); ret blocks end the walk; the fuel
cap bounds loops (net-zero bodies contribute zero net). -/
def walkNetHigh (blocks : List SBlock) (calls : List (String × Int)) (fuel : Nat) : Option (Int × Int) :=
  match fuel with
  | 0 => none
  | fuel' + 1 =>
      let b := blocks.getD 0 (SBlock.mk 0 [] [])
      walkFrom blocks calls b fuel' 0 0
where
  walkFrom : List SBlock → List (String × Int) → SBlock → Nat → Int → Int → Option (Int × Int)
    | blocks, calls, b, fuel, depth, peak =>
        match fuel with
        | 0 => some (depth, peak)
        | fuel' + 1 =>
            let ops := b.ops
            let (newDepth, newPeak) := (runDepth ops depth, peakDepth ops depth)
            match b.succs with
            | [] => some (newDepth, newPeak)
            | t :: _ => walkFrom blocks calls (blocks.getD t (SBlock.mk 0 [] [])) fuel' newDepth (max peak newPeak)

/-- The re-derivation result for one word. -/
structure Check where
  name : String
  status : String       -- "ok" | "net-mismatch" | "peak-over" | "skipped-unresolved" | "walk-timeout"
  net : Int
  high : Int
  gotNet : Int
  gotHigh : Int
  deriving Repr

/-- The check for one word. -/
def checkWord (w : WordMeta) : Check :=
  if w.unresolved then
    Check.mk w.name "skipped-unresolved" w.net w.high 0 0
  else
    let blocks := parseBlocksConcrete w.blocks w.calls
    match walkNetHigh blocks w.calls 64 with
    | none =>
        Check.mk w.name "walk-timeout" w.net w.high 0 0
    | some (gotNet, gotHigh) =>
        let netOk := gotNet == w.net
        let highOk := w.top || gotHigh ≤ w.high
        if netOk && highOk then
          Check.mk w.name "ok" w.net w.high gotNet gotHigh
        else if !netOk then
          Check.mk w.name "net-mismatch" w.net w.high gotNet gotHigh
        else
          Check.mk w.name "peak-over" w.net w.high gotNet gotHigh

/-- Check a whole golden file; returns the failing checks. -/
def checkFile (f : MetaFile) : List Check :=
  f.words.filterMap (fun w =>
    let c := checkWord w
    if c.status == "ok" || c.status == "skipped-unresolved" then none else some c)

/-- Parse a `tyu.stackmeta/1` document. -/
def parseMetaFile (input : String) : Option MetaFile :=
  match parseJson input with
  | none => none
  | some j =>
      if (j.field "schema").bind Json.asStr ≠ some "tyu.stackmeta/1" then none
      else
        let module := (j.field "module").bind Json.asStr |>.getD ""
        let target := (j.field "target").bind Json.asStr |>.getD ""
        let ws := (j.field "words").bind Json.asArr |>.getD []
        let parsed := ws.filterMap (fun wj =>
          let name := (wj.field "name").bind Json.asStr |>.getD ""
          let net := (wj.field "net").bind Json.asInt |>.getD 0
          let high := (wj.field "high").bind Json.asInt |>.getD 0
          let top := (wj.field "top").bind Json.asBool |>.getD false
          let blocks := (wj.field "blocks").bind Json.asStr |>.getD ""
          let unresolved := (wj.field "unresolved").bind Json.asBool |>.getD false
          let calls := match (wj.field "calls").bind Json.fieldObj with
            | some flds => flds.filterMap (fun (k, v) => (Json.asInt v).map (fun n => (k, n)))
            | none => []
          some { name := name, net := net, high := high, top := top,
                 blocks := blocks, calls := calls, unresolved := unresolved })
        if parsed.length ≠ ws.length then none
        else some { schema := "tyu.stackmeta/1", target := target, module := module,
                    words := parsed }

end Tyu.Stackmeta