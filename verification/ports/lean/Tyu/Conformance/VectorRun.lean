import Tyu.Conformance.Parse

namespace Tyu.Conformance

/-- Parse one canonical op line (the text `ir::write_op` prints) into an
`OpInst`. The declared subtype cast (`cast percent`) carries
`subtypeCast := true`. -/
def parseOp (line : String) : Option OpInst :=
  let toks := (splitOnStr ' ' (trim line)).filter (fun t => t ≠ "")
  match toks with
  | [] => none
  | m :: rest =>
      match m with
      | "const_i64" => match rest with
          | [v] => parseInt v |>.map (fun n => (OpInst.opMk .const_i64).setConst n)
          | _ => none
      | "const_bool" => match rest with
          | [b] => some ((OpInst.opMk .const_bool).setConstBool (b == "true"))
          | _ => none
      | "const_str" => some (OpInst.opMk .const_str)
      | "addr_of" | "addr_of_mut" => some (OpInst.opMk (if m == "addr_of" then .addr_of else .addr_of_mut))
      | "mmio_place" => some (OpInst.opMk .mmio_place)
      | "scoped_enter" => some (OpInst.opMk .scoped_enter)
      | "task_spawn" => some (OpInst.opMk .task_spawn)
      | "ptr_add_const" => some (OpInst.opMk .ptr_add_const)
      | "ptr_add_index" => some (OpInst.opMk .ptr_add_index)
      | "dup" => some (OpInst.opMk .dup)
      | "drop" => some (OpInst.opMk .drop)
      | "swap" => some (OpInst.opMk .swap)
      | "add_i64" => some (OpInst.opMk .add_i64)
      | "sub_i64" => some (OpInst.opMk .sub_i64)
      | "mul_i64" => some (OpInst.opMk .mul_i64)
      | "cmp_lt" | "cmp_le" | "cmp_gt" | "cmp_ge" | "cmp_eq" | "cmp_ne" =>
          some (OpInst.opMk (parseCmp m))
      | "and_bool" => some (OpInst.opMk .and_bool)
      | "or_bool" => some (OpInst.opMk .or_bool)
      | "not_bool" => some (OpInst.opMk .not_bool)
      | "interrupt_disable" => some (OpInst.opMk .interrupt_disable)
      | "interrupt_enable" => some (OpInst.opMk .interrupt_enable)
      | "local_set" => match rest with
          | [n] => parseNat n |>.map (fun i => (OpInst.opMk .local_set).setSlot i)
          | _ => none
      | "local_get" => match rest with
          | [n] => parseNat n |>.map (fun i => (OpInst.opMk .local_get).setSlot i)
          | _ => none
      | "cast" => match rest with
          | [ty] => some ((OpInst.opMk .cast).setSubtypeCast (ty == "percent"))
          | _ => none
      | "bitcast" => some (OpInst.opMk .bitcast)
      | "call" => some (OpInst.opMk .call)
      | "load" => some (OpInst.opMk .load)
      | "store" => some (OpInst.opMk .store)
      | "vol_load" => some (OpInst.opMk .vol_load)
      | "vol_store" => some (OpInst.opMk .vol_store)
      | "vol_load_field" => some (OpInst.opMk .vol_load_field)
      | "vol_store_field" => some (OpInst.opMk .vol_store_field)
      | "trap_if_false" => some (OpInst.opMk .trap_if_false)
      | "br" => match rest with
          | [t] => parseBlockId t |>.map (fun i => (OpInst.opMk .br).setBrTgt i)
          | _ => none
      | "br_if" => match rest with
          | [t, e] => match parseBlockId t, parseBlockId e with
              | some a, some b => some ((OpInst.opMk .br_if).setBrIf (a, b))
              | _, _ => none
          | _ => none
      | "ret" => some (OpInst.opMk .ret)
      | _ => none
where
  parseCmp : String → Tyu.IR.OpForm
    | "cmp_lt" => .cmp_lt | "cmp_le" => .cmp_le | "cmp_gt" => .cmp_gt
    | "cmp_ge" => .cmp_ge | "cmp_eq" => .cmp_eq | _ => .cmp_ne
  parseBlockId (t : String) : Option Nat :=
    match dropPrefixStr "b" t with
    | some t' => parseNat t'
    | none => parseNat t

/-- Parse the linear ops text (newline-separated canonical op lines). -/
def parseOps (text : String) : List OpInst :=
  let lines := splitOnStr '\n' text
  lines.filterMap (fun l => if trim l == "" then none else parseOp l)

/-- Parse the CFG blocks text: `block bN` headers plus op lines; derive the
successor edges from the terminator. -/
def parseBlocks (text : String) : List Block :=
  let lines := splitOnStr '\n' text
  let (acc, curId, curOps) := lines.foldl
    (fun (acc : List Block × Option Nat × List OpInst) line =>
      let (blks, cid, cops) := acc
      let t := trim line
      if t == "" then acc
      else if t.startsWith "block b" then
        match dropPrefixStr "block b" t with
        | some rest =>
            match parseNat rest with
            | some i =>
                let blksFinal := match cid with
                  | some id => (Block.mk id cops.reverse (succsOf cops.reverse)) :: blks
                  | none => blks
                (blksFinal, some i, [])
            | none => acc
        | none => acc
      else match parseOp t with
        | some o => (blks, cid, o :: cops)
        | none => acc)
    ([], none, [])
  let final := match curId with
    | some id => (Block.mk id curOps.reverse (succsOf curOps.reverse)) :: acc
    | none => acc
  final.reverse
where
  succsOf (ops : List OpInst) : List Nat :=
    match ops.getLast? with
    | some o => (match o.form, o.brTgt, o.brIfTgts with
        | .br, some t, _ => [t]
        | .br_if, _, some (t, e) => [t, e]
        | _, _, _ => [])
    | none => []
  Block.mk (id : Nat) (ops : List OpInst) (succs : List Nat) : Block := { id := id, ops := ops, succs := succs }

/-- The extracted fields of one `tyu.vec/1` vector. -/
structure Vector where
  id : String
  cls : String
  row : String
  model : String
  mode : String
  ops : String
  entry : String
  sigIn : Nat
  sigOut : Nat
  blocks : String
  targetLo : Int
  targetHi : Int
  castSite : Bool
  expectHead : String
  expectTop : String
  deriving DecidableEq, Repr, Inhabited

/-- Extract a vector from its JSON object (schema-typed; `none` on any
malformed field — the reader fails closed). -/
def vectorFromJson (j : Json) : Option Vector := do
  let id := (j.field "id").bind Json.asStr
  let className := (j.field "class").bind Json.asStr
  let row := (j.field "row").bind Json.asStr
  let model := (j.field "model").bind Json.asStr
  let mode := (j.field "mode").bind Json.asStr
  let targetArr : List Json := (j.field "target").bind Json.asArr |>.getD []
  let castSite := (j.field "cast_site").bind Json.asBool
  let expect := j.field "expect"
  let expectHead := expect.bind (fun e => (e.field "head").bind Json.asStr)
  let expectTop := expect.bind (fun e => (e.field "top").bind Json.asStr)
  match targetArr with
  | [loJ, hiJ] =>
      let lo := (Json.asInt loJ).getD 0
      let hi := (Json.asInt hiJ).getD 0
      let v : Vector := {
        id := id.getD "", cls := className.getD "", row := row.getD "",
        model := model.getD "", mode := mode.getD "",
        ops := (j.field "ops").bind Json.asStr |>.getD "",
        entry := (j.field "entry").bind Json.asStr |>.getD "",
        sigIn := ((j.field "sig_in").bind Json.asInt |>.getD 0).toNat,
        sigOut := ((j.field "sig_out").bind Json.asInt |>.getD 0).toNat,
        blocks := (j.field "blocks").bind Json.asStr |>.getD "",
        targetLo := lo, targetHi := hi, castSite := castSite.getD false,
        expectHead := expectHead.getD "", expectTop := expectTop.getD "" }
      some v
  | _ => none

end Tyu.Conformance
