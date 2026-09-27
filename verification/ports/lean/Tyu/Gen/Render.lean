import Tyu.Gen.Stmt
import Tyu.Gen.Sha256
import Tyu.Src
import Tyu.Conformance.Json
import Tyu.Conformance.Parse

/-! The statement renderer (PLAN-VERIFY-3 P5.1/P5.2).

`Render.renderModule` is the *total, deterministic* function from the artifact
document (`tyu.obl/v2`) to the generated `Gen/<Module>.lean` text plus the
`<Module>.gen.json` metadata. It re-derives each obligation's *canonical
statement* (`tyu.stmt/1.0` — the encoding of `crates/verifier/src/stmt.rs`)
and its `statement_hash` in Lean (this port's own SHA-256), renders the
`def stmt_… : Prop` statements over `Tyu.Gen.Stmt` (word-level out/in-range,
mmio-bounds, contract-predicate claims), and the per-cycle schemes + the
via-cycles composition (P5.2). Statements it cannot express truthfully are
refused and recorded as *omitted* with their reason (opaque `$top` sites,
words containing `call`, dynamic MMIO offsets, `stack-budget`).

The rendering rules are the statement registry's normative interface (§Q4
item 4; spec-grade review material T-F2).
-/

namespace Tyu.Gen.Render

open Tyu.Conformance
open Tyu.Gen
open Tyu.Gen.Stmt

-- ---------------------------------------------------------------------
-- Errors + canonical JSON primitives
-- ---------------------------------------------------------------------

inductive RenderErr where
  | notOblDoc (msg : String)
  | unknownForm (msg : String)
  deriving Repr, Inhabited

abbrev M := Except RenderErr

def escRest (acc : List String) : List Char → String
  | [] => String.join acc.reverse
  | '"' :: r => escRest ("\\\"" :: acc) r
  | '\\' :: r => escRest ("\\\\" :: acc) r
  | '\n' :: r => escRest ("\\n" :: acc) r
  | '\r' :: r => escRest ("\\r" :: acc) r
  | '\t' :: r => escRest ("\\t" :: acc) r
  | c :: r =>
      if c.toNat == 8 then escRest ("\\b" :: acc) r
      else if c.toNat == 12 then escRest ("\\f" :: acc) r
      else if c.toNat < 32 then
        escRest (("\\u00" ++ Tyu.Gen.Sha256.hexOfByte (Tyu.Gen.Sha256.u8 c.toNat)) :: acc) r
      else escRest (String.mk [c] :: acc) r

def jesc (s : String) : String := escRest [] s.toList
def jstr (s : String) : String := "\"" ++ jesc s ++ "\""
def jnum (n : Int) : String := toString n

def jobj (kvs : List (String × String)) : String :=
  "{" ++ String.intercalate "," (kvs.map (fun (k, v) => jstr k ++ ":" ++ v)) ++ "}"
def jarr (xs : List String) : String := "[" ++ String.intercalate "," xs ++ "]"

/-- The canonical OEL value (mirrors `stmt.rs::canon_oel`). `$top` is a
plain `Var` in the canonical form (the formula's opaque args are carried as
`{"name":"$top","op":"Var"}` — never a bare string). -/
partial def oelCanon : Oel → String
  | .inVar i => jobj [ ("name", jstr ("in." ++ toString i)), ("op", jstr "Var") ]
  | .outVar i => jobj [ ("name", jstr ("out." ++ toString i)), ("op", jstr "Var") ]
  | .castArg src dst arg => jobj [ ("arg", oelCanon arg), ("from", jstr src), ("op", jstr "Cast"), ("to", jstr dst) ]
  | .opaqueMk => jobj [ ("name", jstr "$top"), ("op", jstr "Var") ]

/-- `word_ir_hash` ($6.1). -/
def wordIrHash (ir : String) : String :=
  Tyu.Gen.Sha256.sha256Hex16 ir

-- ---------------------------------------------------------------------
-- Statement surface (parsed from the artifact) + canonical statements
-- ---------------------------------------------------------------------

/-- The statement context (§6.2). -/
structure Ctx where
  target : String
  model : String
  module : String
  word : String
  wordIrHash : String
  kind : String
  occurrence : Nat
  refinement : Option String

/-- A Lean-safe identifier: non-Alphanumeric becomes `_`. -/
def ident (s : String) : String :=
  String.ofList (s.toList.map (fun c =>
    if ('a' ≤ c && c ≤ 'z') || ('A' ≤ c && c ≤ 'Z') || ('0' ≤ c && c ≤ '9') || c == '_' then c else '_'))

def stmtName (module word kind : String) (occ : Nat) : String :=
  "stmt_" ++ ident module ++ "_" ++ ident word ++ "_" ++ ident kind ++ "_" ++ toString occ

def wordRef (module word : String) : String := ident module ++ "_" ++ ident word

def cycleName (module word : String) (n : Nat) : String :=
  "cycle_" ++ ident module ++ "_" ++ ident word ++ "_" ++ toString n

def viaName (module word : String) (kind : String) (occ : Nat) : String :=
  stmtName module word kind occ ++ "_via_cycles"

-- ---------------------------------------------------------------------
-- The parsed artifact
-- ---------------------------------------------------------------------

structure Obl where
  id : String
  idHash : String
  kind : String
  word : String
  occurrence : Nat
  cycles : List (Nat × List Nat)
  formulaOp : String
  oel : Oel
  lo : Int
  hi : Int
  off : Option UInt32
  width : UInt32
  size : UInt32
  predModule : String
  predName : String
  predIr : List String
  predIrHash : String
  args : List Oel
  deriving Inhabited

structure WordFact where
  name : String
  ir : String
  deriving Inhabited, BEq, Repr

structure Artifact where
  module : String
  target : String
  modelSemantics : String
  words : List WordFact
  obligations : List Obl
  predicates : List (String × List String × String)
  deriving Inhabited

/-- `$top`-rooted (opaque): the statement cannot be expressed truthfully. -/
partial def oelOpaque : Oel → Bool
  | .opaqueMk => true
  | .castArg _ _ a => oelOpaque a
  | _ => false

/-- The root reference of an OEL: `("in"|"out", index)`. -/
partial def oelRoot : Oel → Option (String × Nat)
  | .inVar i => some ("in", i)
  | .outVar i => some ("out", i)
  | .castArg _ _ a => oelRoot a
  | .opaqueMk => none

partial def parseOel (j : Json) : M Oel :=
  match (j.field "op").bind Json.asStr with
  | some "Var" =>
      match (j.field "name").bind Json.asStr with
      | some n =>
          if n == "$top" then pure Oel.opaqueMk
          else if n.startsWith "in." then
            match parseNat ((dropPrefixStr "in." n).getD "") with
            | some i => pure (.inVar i)
            | none => throw (.unknownForm ("bad in-ref " ++ n))
          else if n.startsWith "out." then
            match parseNat ((dropPrefixStr "out." n).getD "") with
            | some i => pure (.outVar i)
            | none => throw (.unknownForm ("bad out-ref " ++ n))
          else throw (.unknownForm ("opaque/unknown OEL var " ++ n))
      | none => throw (.notOblDoc "OEL Var without name")
  | some "Cast" =>
      let src := (j.field "from").bind Json.asStr |>.getD ""
      let dst := (j.field "to").bind Json.asStr |>.getD ""
      match (j.field "arg") with
      | some a => parseOel a |>.map (fun x => Oel.castArg src dst x)
      | none => throw (.notOblDoc "OEL Cast without arg")
  | _ => throw (.unknownForm "unknown OEL op")

/-- Parse one obligation; an opaque `$top` value is *kept* (the renderer
classifies it as omitted rather than failing the module). -/
def parseObligation (j : Json) : M Obl :=
  let id := (j.field "id").bind Json.asStr |>.getD ""
  let idHash := (j.field "id_hash").bind Json.asStr |>.getD ""
  let kind := (j.field "kind").bind Json.asStr |>.getD ""
  let word := (j.field "site").bind (fun s => (s.field "word").bind Json.asStr) |>.getD ""
  let occurrence := ((j.field "site").bind (fun s => (s.field "occurrence").bind Json.asInt) |>.getD 0).toNat
  let cycles := (j.field "cycles").bind Json.asArr |>.getD []
  let cycleData := cycles.filterMap (fun c =>
    match (c.field "index").bind Json.asInt, (c.field "blocks").bind Json.asArr with
    | some n, some bs =>
        let ids := bs.filterMap (fun b =>
          (Json.asStr b).bind (fun s =>
            let n := match (dropPrefixStr "b" s) with
              | some rest => parseNat rest
              | none => parseNat s
            n))
        if ids.isEmpty then none else some (n.toNat, ids)
    | _, _ => none)
  match (j.field "formula") with
  | some fj =>
      match (fj.field "op").bind Json.asStr with
      | some "InRange" =>
          let lo := (fj.field "lo").bind Json.asInt |>.getD 0
          let hi := (fj.field "hi").bind Json.asInt |>.getD 0
          match (fj.field "value").bind (fun v => (parseOel v).toOption) with
          | some o => pure (Obl.mk id idHash kind word occurrence cycleData "InRange" o lo hi none 0 0 "" "" [] "" [])
          | none => throw (.notOblDoc ("InRange without value: " ++ id))
      | some "OffsetLE" =>
          let width := (fj.field "width").bind Json.asInt |>.getD 0
          let size := (fj.field "size").bind Json.asInt |>.getD 0
          let off := match (fj.field "off") with
            | some n => (Json.asInt n).map (fun v => UInt32.ofNat v.toNat)
            | none => none
          pure (Obl.mk id idHash kind word occurrence cycleData "OffsetLE" Oel.opaqueMk 0 1 off (UInt32.ofNat width.toNat) (UInt32.ofNat size.toNat) "" "" [] "" [])
      | some "PredicateHolds" =>
          let pred := (fj.field "predicate")
          let predModule := pred.bind (fun p => (p.field "module").bind Json.asStr) |>.getD ""
          let predName := pred.bind (fun p => (p.field "name").bind Json.asStr) |>.getD ""
          let predIr := pred.bind (fun p => (p.field "ir").bind Json.asArr) |>.getD [] |>.filterMap Json.asStr
          let predIrHash := pred.bind (fun p => (p.field "ir_hash").bind Json.asStr) |>.getD ""
          let argVals := (fj.field "args").bind Json.asArr |>.getD []
          let args := argVals.filterMap (fun v => (parseOel v).toOption)
          -- fail closed when an arg is unparseable (the canonical must not
          -- silently drop a member)
          if ¬ argVals.all (fun v => (parseOel v).toOption.isSome)
          then throw (.notOblDoc ("PredicateHolds arg unparseable: " ++ id))
          else pure (Obl.mk id idHash kind word occurrence cycleData
                       "PredicateHolds" Oel.opaqueMk 0 1 none 0 0
                       predModule predName predIr predIrHash args)
      | _ => throw (.unknownForm ("obligation " ++ id ++ ": unknown formula op"))
  | none => throw (.notOblDoc ("obligation without formula: " ++ id))

/-- Parse an artifact document. -/
def parseArtifact (input : String) : M Artifact :=
  match parseJson input with
  | none => throw (.notOblDoc "unparseable JSON")
  | some j =>
      let schema := (j.field "schema").bind Json.asStr |>.getD ""
      if schema ≠ "tyu.obl/v2" then throw (.notOblDoc ("schema: " ++ schema))
      else
        let module := (j.field "module").bind Json.asStr |>.getD ""
        let target := (j.field "target").bind Json.asStr |>.getD ""
        let model := (j.field "model_semantics").bind Json.asStr |>.getD ""
        let wordsList := ((j.field "facts").bind (fun f => (f.field "words").bind Json.asArr)) |>.getD []
        let predicatesList := ((j.field "facts").bind (fun f => (f.field "predicates").bind Json.asArr)) |>.getD []
        let words := wordsList.filterMap (fun w =>
            (w.field "name").bind Json.asStr |>.map (fun n =>
              WordFact.mk n ((w.field "ir").bind Json.asStr |>.getD "")))
        let predicates := predicatesList.filterMap (fun p =>
            (p.field "name").bind Json.asStr |>.map (fun n =>
              (n, (p.field "ir").bind Json.asArr |>.getD [] |>.filterMap Json.asStr,
               (p.field "ir_hash").bind Json.asStr |>.getD "")))
        let raw := (j.field "obligations").bind Json.asArr |>.getD []
        let obligations := raw.filterMap (fun o => (parseObligation o).toOption)
        if obligations.length ≠ raw.length then throw (.notOblDoc "an obligation failed to parse — fail closed")
        else pure (Artifact.mk module target model words obligations predicates)

/-- The canonical statement bytes for an obligation (dispatches on the
formula op — mirrors `stmt.rs::push_formula` byte-for-byte). Declared here
(after the `Obl` structure) so field projection resolves. -/
def canonical (ctx : Ctx) (o : Obl) : String :=
  let cobj := jobj [ ("kind", jstr ctx.kind), ("model_semantics", jstr ctx.model)
                   , ("module", jstr ctx.module), ("occurrence", jnum ctx.occurrence)
                   , ("refinement", match ctx.refinement with | some r => jstr r | none => "null")
                   , ("semantics", jstr "tyu.ir-sem/1.0"), ("stmt_ver", jstr "tyu.stmt/1.0")
                   , ("target", jstr ctx.target), ("word", jstr ctx.word)
                   , ("word_ir_hash", jstr ctx.wordIrHash) ]
  let fobj : String := match o.formulaOp with
    | "InRange" =>
        jobj [ ("hi", jnum o.hi), ("lo", jnum o.lo), ("op", jstr "InRange"), ("value", oelCanon o.oel) ]
    | "OffsetLE" =>
        let off := match o.off with | some n => jnum (Int.ofNat n.toNat) | none => "null"
        jobj [ ("off", off), ("op", jstr "OffsetLE"), ("size", jnum (Int.ofNat o.size.toNat)), ("width", jnum (Int.ofNat o.width.toNat)) ]
    | "PredicateHolds" =>
        let args := jarr (o.args.map oelCanon)
        let pred := jobj [ ("ir", jarr (o.predIr.map jstr)), ("ir_hash", jstr o.predIrHash)
                         , ("module", jstr o.predModule), ("name", jstr o.predName) ]
        jobj [ ("args", args), ("op", jstr "PredicateHolds"), ("predicate", pred) ]
    | _ => "null"
  "{\"context\":" ++ cobj ++ ",\"formula\":" ++ fobj ++ "}"

/-- `statement_hash` (§6.2). -/
def statementHash (ctx : Ctx) (o : Obl) : String :=
  Tyu.Gen.Sha256.sha256Hex (canonical ctx o)

/-- Render one canonical op line as a Lean `ConcreteOp` literal. -/
def renderOp : String → Option String
  | line =>
      let toks := (splitOnStr ' ' (trim line)).filter (fun t => t ≠ "")
      let opMk (f : String) : String := "(Tyu.Step.ConcreteOp.opMk ." ++ f ++ ")"
      match toks with
      | [] => none
      | m :: rest =>
          match m with
          | "const_i64" => (match rest with | [v] => (parseInt v).map (fun n => opMk "const_i64" ++ ".setConst (" ++ toString n ++ ")") | _ => none)
          | "const_bool" => (match rest with | [b] => some (opMk "const_bool" ++ ".setConstBool (" ++ (if b == "true" then "true" else "false") ++ ")") | _ => none)
          | "const_str" => some (opMk "const_str")
          | "addr_of" => some (opMk "addr_of")
          | "addr_of_mut" => some (opMk "addr_of_mut")
          | "mmio_place" => some (opMk "mmio_place")
          | "scoped_enter" => some (opMk "scoped_enter")
          | "task_spawn" => some (opMk "task_spawn")
          | "ptr_add_const" => some (opMk "ptr_add_const")
          | "ptr_add_index" => some (opMk "ptr_add_index")
          | "dup" => some (opMk "dup")
          | "drop" => some (opMk "drop")
          | "swap" => some (opMk "swap")
          | "add_i64" => some (opMk "add_i64")
          | "sub_i64" => some (opMk "sub_i64")
          | "mul_i64" => some (opMk "mul_i64")
          | "cmp_lt" => some (opMk "cmp_lt")
          | "cmp_le" => some (opMk "cmp_le")
          | "cmp_gt" => some (opMk "cmp_gt")
          | "cmp_ge" => some (opMk "cmp_ge")
          | "cmp_eq" => some (opMk "cmp_eq")
          | "cmp_ne" => some (opMk "cmp_ne")
          | "and_bool" => some (opMk "and_bool")
          | "or_bool" => some (opMk "or_bool")
          | "not_bool" => some (opMk "not_bool")
          | "interrupt_disable" => some (opMk "interrupt_disable")
          | "interrupt_enable" => some (opMk "interrupt_enable")
          | "local_set" => (match rest with | [n] => (parseNat n).map (fun i => opMk "local_set" ++ ".setSlot (" ++ toString i ++ ")") | _ => none)
          | "local_get" => (match rest with | [n] => (parseNat n).map (fun i => opMk "local_get" ++ ".setSlot (" ++ toString i ++ ")") | _ => none)
          | "cast" => some (opMk "cast")
          | "bitcast" => some (opMk "bitcast")
          | "call" => some (opMk "call")
          | "load" => some (opMk "load")
          | "store" => some (opMk "store")
          | "vol_load" => some (opMk "vol_load")
          | "vol_store" => some (opMk "vol_store")
          | "vol_load_field" => some (opMk "vol_load_field")
          | "vol_store_field" => some (opMk "vol_store_field")
          | "trap_if_false" => some (opMk "trap_if_false")
          | "br" => (match rest with | [t] => ((dropPrefixStr "b" t).bind parseNat).map (fun i => opMk "br" ++ ".setBrTgt (" ++ toString i ++ ")") | _ => none)
          | "br_if" => (match rest with
              | [t, e] => match (dropPrefixStr "b" t).bind parseNat, (dropPrefixStr "b" e).bind parseNat with
                  | some a, some b => some (opMk "br_if" ++ ".setBrIf (" ++ toString a ++ ", " ++ toString b ++ ")")
                  | _, _ => none
              | _ => none)
          | "ret" => some (opMk "ret")
          | _ => none

/-! ### The source-surface statement forms (PLAN-VERIFY-3 P9.1)

A renderable obligation MAY additionally be stated over the pure-fragment
*source* semantics (`Tyu/Src.lean`), when its word's canonical op text is
entirely fragment ops (the op-locality test: every line parses via
`parseSrcOp`). The statement is `def src_stmt_<name> : Prop := …` — the same
claim evaluated under `Tyu.Src`; a theorem of it is harvested with
`surface: "source"` and `relies: ["T-S"]` (§Q2, §6.3). Words with casts,
trap sequences, calls, or address ops fail the test and stay IR-surface
(honest refusal — the fragment boundary of `Tyu/Src.lean`). -/

/-- Parse a fragment op line into a `Tyu.Src.Op` (`none` = not a fragment op
— the op-locality detector). -/
def parseSrcOp (line : String) : Option Tyu.Src.Op :=
  let toks := (splitOnStr ' ' (trim line)).filter (fun t => t ≠ "")
  match toks with
  | [] => none
  | m :: rest =>
      match m with
      | "const_i64" => (match rest with | [v] => (parseInt v).map (fun n => Tyu.Src.Op.constInt n) | _ => none)
      | "const_bool" => (match rest with | [b] => (if b == "true" then some (Tyu.Src.Op.constBool true) else some (Tyu.Src.Op.constBool false)) | _ => none)
      | "dup" => some Tyu.Src.Op.dup
      | "drop" => some Tyu.Src.Op.drop
      | "swap" => some Tyu.Src.Op.swap
      | "add_i64" => some Tyu.Src.Op.add
      | "sub_i64" => some Tyu.Src.Op.sub
      | "mul_i64" => some Tyu.Src.Op.mul
      | "cmp_lt" => some Tyu.Src.Op.cmpLT
      | "cmp_le" => some Tyu.Src.Op.cmpLE
      | "cmp_gt" => some Tyu.Src.Op.cmpGT
      | "cmp_ge" => some Tyu.Src.Op.cmpGE
      | "cmp_eq" => some Tyu.Src.Op.cmpEQ
      | "cmp_ne" => some Tyu.Src.Op.cmpNE
      | "and_bool" => some Tyu.Src.Op.andB
      | "or_bool" => some Tyu.Src.Op.orB
      | "not_bool" => some Tyu.Src.Op.notB
      | "load" => some Tyu.Src.Op.load
      | "store" => some Tyu.Src.Op.store
      | "vol_load" => some Tyu.Src.Op.volLoad
      | "vol_store" => some Tyu.Src.Op.volStore
      | "local_get" => (match rest with | [n] => (parseNat n).map (fun i => Tyu.Src.Op.localGet i) | _ => none)
      | "local_set" => (match rest with | [n] => (parseNat n).map (fun i => Tyu.Src.Op.localSet i) | _ => none)
      | "br" => (match rest with | [t] => ((dropPrefixStr "b" t).bind parseNat).map (fun i => Tyu.Src.Op.br i) | _ => none)
      | "br_if" => (match rest with
          | [t, e] => match (dropPrefixStr "b" t).bind parseNat, (dropPrefixStr "b" e).bind parseNat with
              | some a, some b => some (Tyu.Src.Op.brIf a b)
              | _, _ => none
          | _ => none)
      | "ret" => some Tyu.Src.Op.ret
      | _ => none

/-- Parse a word's canonical op text into source-fragment blocks (`none`
when any op is outside the fragment — the op-locality detector). -/
def parseSrcBlocks (ir : String) : Option (List Tyu.Src.Block) :=
  let lines := (splitOnStr '\n' ir).map trim |>.filter (fun l => l ≠ "")
  let rec go (rest : List String) (blocks : List Tyu.Src.Block) (cid : Option Nat) (ops : List Tyu.Src.Op) : Option (List Tyu.Src.Block) :=
    match rest with
    | [] =>
        match cid with
        | some c => some (({ id := c, ops := ops.reverse } : Tyu.Src.Block) :: blocks).reverse
        | none => some blocks.reverse
    | line :: rest =>
        match (dropPrefixStr "block b" line).bind parseNat with
        | some i =>
            let blocks' := match cid with
              | some c => { id := c, ops := ops.reverse } :: blocks
              | none => blocks
            go rest blocks' (some i) []
        | none =>
            match parseSrcOp line with
            | some o => go rest blocks cid (o :: ops)
            | none => none
  go lines [] none []

/-- Render a source-fragment block list as a Lean literal. -/
def renderSrcBlocks (blocks : List Tyu.Src.Block) : String :=
  "[ " ++ String.intercalate ", " (blocks.map (fun b =>
    "{ id := " ++ toString b.id ++ ", ops := [ " ++ String.intercalate ", " (b.ops.map renderSrcOp) ++ " ] }")) ++ " ]"
where
  renderSrcOp (o : Tyu.Src.Op) : String :=
    match o with
    | Tyu.Src.Op.constInt v => "(Tyu.Src.Op.constInt " ++ toString v ++ ")"
    | Tyu.Src.Op.constBool b => "(Tyu.Src.Op.constBool " ++ (if b then "true" else "false") ++ ")"
    | Tyu.Src.Op.dup => "Tyu.Src.Op.dup"
    | Tyu.Src.Op.drop => "Tyu.Src.Op.drop"
    | Tyu.Src.Op.swap => "Tyu.Src.Op.swap"
    | Tyu.Src.Op.add => "Tyu.Src.Op.add"
    | Tyu.Src.Op.sub => "Tyu.Src.Op.sub"
    | Tyu.Src.Op.mul => "Tyu.Src.Op.mul"
    | Tyu.Src.Op.cmpLT => "Tyu.Src.Op.cmpLT"
    | Tyu.Src.Op.cmpLE => "Tyu.Src.Op.cmpLE"
    | Tyu.Src.Op.cmpGT => "Tyu.Src.Op.cmpGT"
    | Tyu.Src.Op.cmpGE => "Tyu.Src.Op.cmpGE"
    | Tyu.Src.Op.cmpEQ => "Tyu.Src.Op.cmpEQ"
    | Tyu.Src.Op.cmpNE => "Tyu.Src.Op.cmpNE"
    | Tyu.Src.Op.andB => "Tyu.Src.Op.andB"
    | Tyu.Src.Op.orB => "Tyu.Src.Op.orB"
    | Tyu.Src.Op.notB => "Tyu.Src.Op.notB"
    | Tyu.Src.Op.load => "Tyu.Src.Op.load"
    | Tyu.Src.Op.store => "Tyu.Src.Op.store"
    | Tyu.Src.Op.volLoad => "Tyu.Src.Op.volLoad"
    | Tyu.Src.Op.volStore => "Tyu.Src.Op.volStore"
    | Tyu.Src.Op.localGet n => "(Tyu.Src.Op.localGet " ++ toString n ++ ")"
    | Tyu.Src.Op.localSet n => "(Tyu.Src.Op.localSet " ++ toString n ++ ")"
    | Tyu.Src.Op.br t => "(Tyu.Src.Op.br " ++ toString t ++ ")"
    | Tyu.Src.Op.brIf t e => "(Tyu.Src.Op.brIf " ++ toString t ++ " " ++ toString e ++ ")"
    | Tyu.Src.Op.ret => "Tyu.Src.Op.ret"

/-- The source name of a statement: `src_stmt_<module>_<word>_<kind>_<occ>`. -/
def srcStmtName (module word kind : String) (occ : Nat) : String :=
  "src_" ++ stmtName module word kind occ

/-- Whether a word's IR contains a `call` op (unfaithful in the concrete
step's default-sig approximation — such words are `calls-unmodeled`). -/
def wordHasCall (ir : String) : Bool :=
  (splitOnStr '\n' ir).any (fun l => (trim l).startsWith "call ")

/-- Render the blocks text of a word as a Lean `List Tyu.Step.Block`. -/
def renderBlocks (ir : String) : Option String :=
  let lines := (splitOnStr '\n' ir).map trim |>.filter (fun l => l ≠ "")
  let rec go (rest : List String) (blocks : List String) (cid : Option Nat) (ops : List String) : Option String :=
    match rest with
    | [] =>
        let blocks' :=
          match cid with
          | some c => blockLit c ops :: blocks
          | none => blocks
        some ("[ " ++ String.intercalate ", " blocks'.reverse ++ " ]")
    | line :: rest =>
        match (dropPrefixStr "block b" line).bind parseNat with
        | some i =>
            let blocks' := match cid with
              | some c => blockLit c ops :: blocks
              | none => blocks
            go rest blocks' (some i) []
        | none =>
            match renderOp line with
            | some o => go rest blocks cid (ops.concat o)
            | none => none
  go lines [] none []
  where
    blockLit (c : Nat) (ops : List String) : String :=
      "{ id := " ++ toString c ++ ", ops := [ " ++ String.intercalate ", " ops ++ " ] }"

-- ---------------------------------------------------------------------
-- Omission classification
-- ---------------------------------------------------------------------

/-- The predicate's canonical op-text: the transcluded `PredicateRef.ir`
when present, else the module's own `facts.predicates` IR (the callee's own
clause — its artifact was still being written during its own extraction, so
the ref is empty but the facts carry it). -/
def predIrOf (a : Artifact) (o : Obl) : List String :=
  if o.predIr.isEmpty then
    match a.predicates.find? (fun (n, _, _) => n == o.predName) with
    | some (_, ir, _) => ir
    | none => []
  else o.predIr

/-- Every `PredicateHolds` arg must be a plain `in.i`/`out.i` ref for the
exit-state claim to be truthful (opaque and cast chains are refused). -/
def argsPlain (o : Obl) : Bool :=
  o.args.all (fun a => match a with | .inVar _ | .outVar _ => true | _ => false)

/-- Contract obligations render truthfully when:
  1. the predicate IR is resolvable (ref or the module's facts);
  2. the predicate contains no `call` (unfaithful in the statement-side
     step);
  3. for `contract-post` the formula args are concrete `in/out` refs
     (`contract-pre` args are the caller's `$top` values — the statement
     is the ∀-scheme over them, a sound over-claim). -/
def contractClassify (a : Artifact) (o : Obl) : Option String :=
  let ir := predIrOf a o
  if ir.isEmpty then some "predicate-unavailable"
  else if wordHasCall (String.intercalate "\n" ir) then some "predicate-calls-unmodeled"
  else if o.kind == "contract-post" && ¬ argsPlain o then some "opaque-args"
  else none

/-- The statement form of an obligation: which Prop renderer it uses, or
`none` when it must be omitted (with a reason). -/
def classify (a : Artifact) (o : Obl) : Option String :=
  let wcall := match a.words.find? (fun w => w.name == o.word) with
    | some w => wordHasCall w.ir
    | none => false
  if o.kind == "stack-budget" then some "stack-budget-not-rendered"
  else if wcall then some "calls-unmodeled"
  else match o.kind with
    | "subtype-range" => if oelOpaque o.oel then some "opaque-site" else none
    | "mmio-bounds" =>
        if o.formulaOp ≠ "OffsetLE" || o.off.isNone then some "dynamic-offset" else none
    | "contract-pre" | "contract-post" => contractClassify a o
    | _ => some "unknown-kind"

-- ---------------------------------------------------------------------
-- Statement text rendering
-- ---------------------------------------------------------------------

/-- The word-name used in a statement. -/
def wordName (a : Artifact) (o : Obl) : String :=
  wordRef a.module o.word

/-- The contract statements (PredicateHolds).

- `contract-post`: every terminating run's exit outputs satisfy the
  predicate — `∀ σ₀ σf, run w … σ₀ = some σf → predicateHolds wpred … [out args]`;
- `contract-pre`: the predicate holds over the call-site argument tuple —
  rendered as the ∀-scheme over the values (a sound over-claim; the
  compiler records `$top` values for the caller's arguments). -/
def renderPredicateHolds (a : Artifact) (o : Obl) (w : String) : String :=
  let predW := wordRef a.module o.predName
  let binders := " (spec : Tyu.IR.TargetSpec) (mem : Tyu.Step.ConcreteMem) (fuel : Nat)"
  if o.kind == "contract-post" then
    let args := o.args.filterMap (fun arg =>
      match arg with
      | .outVar i => some ("Tyu.Gen.Stmt.outputAt σf " ++ toString i)
      | .inVar i => some ("Tyu.Gen.Stmt.inputAt σ₀ " ++ toString i)
      | _ => none)
    "∀" ++ binders ++ " (σ₀ σf : Tyu.Step.State),\n    " ++
      "Tyu.Gen.Stmt.Word.run " ++ w ++ " spec fuel mem σ₀ = some σf →\n    " ++
      "Tyu.Gen.Stmt.predicateHolds " ++ predW ++ " spec mem fuel [ " ++ String.intercalate ", " args ++ " ]"
  else
    let args := o.args.length
    let names := (List.range args).map (fun i => "a_" ++ toString i)
    let argBinders := names.map (fun n => " (" ++ n ++ " : Int)") |> String.intercalate ""
    "∀" ++ binders ++ argBinders ++ ",\n    " ++
      "Tyu.Gen.Stmt.predicateHolds " ++ predW ++ " spec mem fuel [ " ++ String.intercalate ", " names ++ " ]"

/-- Render the `def … : Prop := …` statement text for a renderable
obligation. -/
def renderStatement (a : Artifact) (o : Obl) : String :=
  let ctx := Ctx.mk a.target a.modelSemantics a.module o.word (wordIrHash (tagList a o)) o.kind o.occurrence none
  let hash := statementHash ctx o
  let name := stmtName a.module o.word o.kind o.occurrence
  let w := wordName a o
  let prop : String := match o.formulaOp with
    | "InRange" =>
        match oelRoot o.oel with
        | some ("in", i) => "Tyu.Gen.Stmt.inInputRange " ++ toString i ++ " (" ++ toString o.lo ++ ") (" ++ toString o.hi ++ ")"
        | some ("out", i) => "Tyu.Gen.Stmt.outInRange " ++ w ++ " " ++ toString i ++ " (" ++ toString o.lo ++ ") (" ++ toString o.hi ++ ")"
        | _ => "True"
    | "OffsetLE" =>
        "Tyu.Gen.Stmt.offsetWithin " ++ w ++ " " ++ toString (o.off.getD 0) ++
          " (" ++ toString o.width ++ ") (" ++ toString o.size ++ ")"
    | "PredicateHolds" => renderPredicateHolds a o w
    | _ => "True"
  "/-- statement: " ++ o.id ++ "\n    statement_hash: " ++ hash ++ " -/\n" ++ "def " ++ name ++ " : Prop :=\n  " ++ prop
where
  tagList (a : Artifact) (o : Obl) : String :=
    (match a.words.find? (fun w => w.name == o.word) with | some w => w.ir | none => "")

/-- The per-cycle scheme (P5.2). -/
def renderCycleScheme (a : Artifact) (word : String) (cyc : Nat × List Nat) : String :=
  let name := cycleName a.module word cyc.1
  let cycTex := "{ index := " ++ toString cyc.1 ++ ", members := [" ++ String.intercalate ", " (cyc.2.map toString) ++ "] }"
  "/-- cycle scheme (P5.2): every member-to-member step of cycle " ++ toString cyc.1 ++
    " preserves the developer's invariant `Inv`. -/\n" ++
  "def " ++ name ++ " : Prop :=\n" ++
  "  ∀ (spec : Tyu.IR.TargetSpec) (mem : Tyu.Step.ConcreteMem) (Inv : Tyu.Step.State → Prop)\n" ++
  "        (b t : Nat) (σ σ' : Tyu.Step.State),\n" ++
  "    Tyu.Gen.Stmt.Member (" ++ cycTex ++ ") b → Tyu.Gen.Stmt.Member (" ++ cycTex ++ ") t →\n" ++
  "    Inv σ → Tyu.Gen.Stmt.StepTo " ++ wordRef a.module word ++ " (" ++ cycTex ++ ") spec mem b t σ σ' →\n" ++
  "    Inv σ'"

/-- The via-cycles composition (P5.2), for a renderable out-range
obligation of a cycle-bearing word. -/
def renderVia (a : Artifact) (o : Obl) (cyc : Nat × List Nat) (i : Nat) (lo hi : Int) : String :=
  let name := viaName a.module o.word o.kind o.occurrence
  let cycTex := "{ index := " ++ toString cyc.1 ++ ", members := [" ++ String.intercalate ", " (cyc.2.map toString) ++ "] }"
  "/-- via-cycles composition (P5.2): the word statement follows from the\n" ++
  "    invariant hypotheses of `Tyu.Gen.Stmt.ViaCycles`. -/\n" ++
  "def " ++ name ++ " : Prop :=\n  " ++
  "Tyu.Gen.Stmt.ViaCycles " ++ wordRef a.module o.word ++ " (" ++ cycTex ++ ") " ++ toString i ++
  " (" ++ toString lo ++ ") (" ++ toString hi ++ ")"

/-- The word IR text of an obligation's word. -/
def wordIrOf (a : Artifact) (o : Obl) : String :=
  match a.words.find? (fun w => w.name == o.word) with
  | some w => w.ir
  | none => ""
/-! ### Source-surface statements (P9.1) -/

/-- Is an obligation renderable at the SOURCE surface? `none` = yes (the
word's IR is entirely fragment ops AND the IR statement itself renders);
`some reason` = refused (non-fragment word, opaque site, contract predicates
— each honest, the fragment boundary of `Tyu/Src.lean`). -/
def srcClassify (a : Artifact) (o : Obl) : Option String :=
  match classify a o with
  | some r => some r
  | none =>
      if o.formulaOp == "PredicateHolds" then some "predicate-source-unavailable"
      else
        let ir := wordIrOf a o
        if wordHasCall ir then some "calls-unmodeled"
        else match parseSrcBlocks ir with
          | some _ => none
          | none => some "non-fragment-ops"

/-- The `def src_stmt_… : Prop := …` text for a source-renderable
obligation (the same claim over the `Tyu.Src` run). -/
def renderSrcStatement (a : Artifact) (o : Obl) : String :=
  let name := srcStmtName a.module o.word o.kind o.occurrence
  let blocks := (parseSrcBlocks (wordIrOf a o)).getD []
  let blocksTex := renderSrcBlocks blocks
  let prop : String := match o.formulaOp with
    | "InRange" =>
        match oelRoot o.oel with
        | some ("in", i) =>
            "Tyu.Src.inInputRange " ++ toString i ++ " (" ++ toString o.lo ++ ") (" ++ toString o.hi ++ ")"
        | some ("out", i) =>
            "Tyu.Src.outInRange (" ++ blocksTex ++ ") 0 " ++ toString i ++ " (" ++ toString o.lo ++ ") (" ++ toString o.hi ++ ")"
        | _ => "True"
    | "OffsetLE" =>
        "Tyu.Src.offsetWithin (" ++ blocksTex ++ ") 0 " ++ toString (o.off.getD 0) ++
          " (" ++ toString o.width ++ ") (" ++ toString o.size ++ ")"
    | _ => "True"
  "/-- source-surface statement (P9.1, §Q2): " ++ o.id ++ "\n    the same claim over the pure-fragment source semantics; a theorem of this\n    statement is harvested with `surface: \"source\"` and `relies: [\"T-S\"]`. -/\n" ++
  "def " ++ name ++ " : Prop :=\n  " ++ prop

/-- The module's source-surface statements (concatenated; empty when no
obligation is source-renderable). -/
def renderSourceStatements (a : Artifact) : String :=
  String.intercalate "\n" (a.obligations.filterMap (fun o =>
    match srcClassify a o with
    | none => some (renderSrcStatement a o)
    | some _ => none))


-- ---------------------------------------------------------------------
-- Per-module assembly: the Gen file text + the Gen metadata json
-- ---------------------------------------------------------------------

/-- The module's renderable statements (text). -/
def renderAllStatements (a : Artifact) : String :=
  String.intercalate "\n" (a.obligations.filterMap (fun o =>
    match classify a o with
    | none => some (renderStatement a o)
    | some _ => none))

/-- The word-blocks defs referenced by renderable statements. -/
def renderWordDefs (a : Artifact) : String :=
  let used := a.obligations.filterMap (fun o => match classify a o with
    | none => a.words.find? (fun w => w.name == o.word)
    | some _ => none) |>.eraseDups
  String.intercalate "\n" (used.map (fun w =>
    "/-- the word's CFG blocks (from the artifact's canonical op text) -/\n" ++
    "def " ++ wordRef a.module w.name ++ "_blocks : List Tyu.Step.Block := " ++
    (renderBlocks w.ir).getD "[]" ++ "\n" ++
    "def " ++ wordRef a.module w.name ++ " : Tyu.Gen.Stmt.Word := { blocks := " ++
    wordRef a.module w.name ++ "_blocks, entry := 0 }"))

/-- The contract-predicate block defs referenced by renderable contract
statements (resolved from the transcluded ref or the module's
`facts.predicates`). A predicate that is itself a rendered word (its own
obligations rendered, so `renderWordDefs` emitted its `Word` def) is
skipped — one definition per name. -/
def renderPredicateDefs (a : Artifact) (skip : List String) : String :=
  let used := a.obligations.filterMap (fun o =>
    match classify a o with
    | none => if o.formulaOp == "PredicateHolds"
                then let ir := predIrOf a o; if ir.isEmpty then none else some (o.predName, ir)
                else none
    | some _ => none) |>.eraseDups
  String.intercalate "\n" ((used.filter (fun (pn, _) => ¬ skip.contains pn)).map (fun (pn, ir) =>
    "/-- the contract predicate's CFG blocks (facts.predicates / transcluded ref) -/\n" ++
    "def " ++ wordRef a.module pn ++ "_blocks : List Tyu.Step.Block := " ++
    (renderBlocks (String.intercalate "\n" ir)).getD "[]" ++ "\n" ++
    "def " ++ wordRef a.module pn ++ " : Tyu.Gen.Stmt.Word := { blocks := " ++
    wordRef a.module pn ++ "_blocks, entry := 0 }"))

/-- The cycle schemes + via-cycles compositions (P5.2). -/
def renderCycles (a : Artifact) : String :=
  let words := a.obligations.filterMap (fun o => a.words.find? (fun w => w.name == o.word)) |>.eraseDups
  let schemes := words.flatMap (fun w =>
    let cycs := (a.obligations.filter (fun o => o.word == w.name)).flatMap (fun o => o.cycles) |>.eraseDups
    cycs.map (fun c => renderCycleScheme a w.name c))
  let vias := a.obligations.filterMap (fun o =>
    match classify a o, oelRoot o.oel with
    | none, some ("out", i) =>
        let cycs := o.cycles
        match cycs.head? with
        | some c => some (renderVia a o c i o.lo o.hi)
        | none => none
    | _, _ => none)
  String.intercalate "\n" (schemes ++ vias)

/-- The `Gen/<Module>.lean` text of a module. -/
def renderModuleLean (a : Artifact) : String :=
  let header := "-- Generated by the Lean port's `gen` renderer (PLAN-VERIFY-3 P5).\n" ++
                "-- DO NOT EDIT — regenerated from `tyu.obl/v2`; statement hashes are\n" ++
                "-- the renderer↔encoder drift lock (crates/tooling-tests).\n\n" ++
                "import Tyu.Gen.Stmt\nimport Tyu.Src\n\n" ++
                "namespace Tyu.Gen.Corpus." ++ ident a.module ++ "\n\n"
  let words := renderWordDefs a
  let wordNames := a.obligations.filterMap (fun o =>
    match classify a o with
    | none => some o.word
    | some _ => none)
  let preds := renderPredicateDefs a wordNames
  let stmts := renderAllStatements a
  let cycs := renderCycles a
  let srcStmts := renderSourceStatements a
  let tail := "\n\nend Tyu.Gen.Corpus." ++ ident a.module ++ "\n"
  header ++ words ++ (if words == "" then "" else "\n\n")
    ++ preds ++ (if preds == "" then "" else "\n\n")
    ++ stmts ++ (if stmts == "" then "" else "\n\n")
    ++ cycs ++ (if srcStmts == "" then "" else "\n\n")
    ++ srcStmts ++ tail



/-- The `<Module>.gen.json` metadata: statements (name+hash) and the
omitted set (id + reason). -/
def renderModuleMeta (a : Artifact) : String :=
  let stmtRows := a.obligations.filterMap (fun o =>
    let id := o.id
    let hash := match classify a o with
      | none =>
          let ctx := Ctx.mk a.target a.modelSemantics a.module o.word (wordIrHash (wordIrOf a o)) o.kind o.occurrence none
          statementHash ctx o
      | some _ => ""
    let omitField := match classify a o with
      | none => "\"def\": \"" ++ stmtName a.module o.word o.kind o.occurrence ++ "\", \"omitted\": false"
      | some reason => "\"omitted\": true, \"reason\": \"" ++ jesc reason ++ "\""
    let srcField := match srcClassify a o with
      | none => ", \"src_def\": \"" ++ srcStmtName a.module o.word o.kind o.occurrence ++ "\""
      | some _ => ""
    some ("  { \"id\": \"" ++ jesc id ++ "\", \"id_hash\": \"" ++ jesc o.idHash ++ "\", " ++ omitField ++ ", \"statement_hash\": \"" ++ hash ++ "\"" ++ srcField ++ " }"))
  let usedWords := a.obligations.filterMap (fun o => match classify a o with
    | none => a.words.find? (fun w => w.name == o.word)
    | some _ => none) |>.eraseDups
  let wordRows := usedWords.map (fun w =>
    let cycs := (a.obligations.filter (fun o => o.word == w.name)).flatMap (fun o => o.cycles) |>.eraseDups
    "  { \"name\": \"" ++ jesc w.name ++ "\", \"def\": \"" ++ wordRef a.module w.name ++ "\", \"entry\": 0, \"cycles\": [" ++
    String.intercalate ", " (cycs.map (fun c =>
      "{ \"index\": " ++ toString c.1 ++ ", \"blocks\": [" ++ String.intercalate ", " (c.2.map (fun b => "\"b" ++ toString b ++ "\"")) ++ "] }")) ++ "] }")
  "{\n  \"schema\": \"tyu.gen/1\",\n  \"module\": \"" ++ jesc a.module ++ "\",\n  \"target\": \"" ++ jesc a.target ++
    "\",\n  \"model_semantics\": \"" ++ jesc a.modelSemantics ++ "\",\n  \"statements\": [\n" ++
    String.intercalate ",\n" stmtRows ++ "\n  ],\n  \"words\": [\n" ++
    String.intercalate ",\n" wordRows ++ "\n  ]\n}\n"

/-- The render driver for one artifact document. -/
def renderModule (doc : String) : M (String × String) := do
  let a ← parseArtifact doc
  pure (renderModuleLean a, renderModuleMeta a)

/-- The SHA-256 known-answer self-check vectors. -/
def selfcheck : List (String × String) :=
  [ ("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
  , ("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
  , ("The quick brown fox jumps over the lazy dog", "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592") ]

end Tyu.Gen.Render
