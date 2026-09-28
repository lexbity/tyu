import Tyu.Conformance.Step

namespace Tyu.Conformance

/-- `mapIdx` (local to Cfg; `Step.mapIdx` exists too but Cfg should not
depend on Step's leaf for a three-line helper). -/
def mapIdxN {α : Type} (l : List α) (f : Nat → α → α) : List α :=
  let rec go : Nat → List α → List α
    | _, [] => []
    | i, x :: xs => f i x :: go (i + 1) xs
  go 0 l


/-- One block of the word CFG: its id, op list, and terminator successors. -/
structure Block where
  id : Nat
  ops : List OpInst
  succs : List Nat
  deriving DecidableEq, Repr, Inhabited

namespace Block

def empty : Block := { id := 0, ops := [], succs := [] }

end Block

/-- Run `ops` through a state (the per-block transfer); the updated model
rides along (`store`/`load` mutate and consult it, P12.2). -/
def runOps (ops : List OpInst) (st : State) (sr : Option (Int × Int)) (widthBits : Nat) (mem : MemModel) : MemModel × State :=
  ops.foldl (fun (acc : MemModel × State) o =>
    let (m, s) := acc
    stepOp o s sr widthBits m) (mem, st)

/-- The two-state hull join (`verifier::interp::hull_state`): per-index join
of stack positions up to `stackLen`; locals everywhere; stack beyond
`stackLen` dropped. -/
def hullState (a b : State) (stackLen : Nat) : State :=
  let n := min stackLen (min a.stack.length b.stack.length)
  let stack := List.ofFn (fun i : Fin n =>
    Slot.computed ((a.stack.getD i.1 Slot.top).iv.join (b.stack.getD i.1 Slot.top).iv))
  let cap := max a.locals.length b.locals.length
  let locals := List.ofFn (fun i : Fin cap =>
    Slot.computed ((a.locals.getD i.1 Slot.top).iv.join (b.locals.getD i.1 Slot.top).iv))
  { stack := stack, locals := locals }

/-- merge_first_visit (`verifier::interp::merge_first_visit`): missing stack
slots are bottom (join identity); missing locals are top. -/
def mergeFirstVisit (old incoming : State) : State :=
  let maxStack := max old.stack.length incoming.stack.length
  let stack := List.ofFn (fun i : Fin maxStack =>
    Slot.computed ((old.stack.getD i.1 (Slot.computed Interval.bottom)).iv.join
                   (incoming.stack.getD i.1 (Slot.computed Interval.bottom)).iv))
  let cap := max old.locals.length incoming.locals.length
  let locals := List.ofFn (fun i : Fin cap =>
    Slot.computed ((old.locals.getD i.1 Slot.top).iv.join (incoming.locals.getD i.1 Slot.top).iv))
  { stack := stack, locals := locals }

/-- Back-edge widening (`verifier::interp::widen_state`): per slot, keep the
old value when the new is a subset of it, else widen to top. -/
def widenState (header bodyEnd : State) (stackLen : Nat) : State :=
  let n := min stackLen (min header.stack.length bodyEnd.stack.length)
  let stack := List.ofFn (fun i : Fin n =>
    let old := header.stack.getD i.1 Slot.top
    let new := bodyEnd.stack.getD i.1 Slot.top
    Slot.computed (if new.iv.subsetOf old.iv then old.iv else Interval.top))
  let cap := max header.locals.length bodyEnd.locals.length
  let locals := List.ofFn (fun i : Fin cap =>
    let old := header.locals.getD i.1 Slot.top
    let new := bodyEnd.locals.getD i.1 Slot.top
    Slot.computed (if new.iv.subsetOf old.iv then old.iv else Interval.top))
  { stack := stack, locals := locals }

/-- The worklist fixpoint body (`verifier::interp::run_cfg`'s loop): structurally
recursive over the fuel list (`List.range budget0`; one element = one pop,
mirroring Rust's `budget -= 1` per iteration). States and visits are plain
lists indexed by block id (core-only `getD`), set with `mapIdxN`. The model
is threaded like the Rust engine's `&mut mem`: each block's run reads and
mutates the single model instance (P12.2's recorded stores persist across
the fixpoint). -/
def runCfgLoop (blocks : List Block) (sigOut : Nat) (sr : Option (Int × Int)) (widthBits : Nat) (mem : MemModel)
    (states : List State) (worklist : List Nat) (visits : List Nat) (fuel : List Nat) : MemModel × List State :=
  match fuel with
  | [] => (mem, states)
  | _ :: frest =>
      match worklist with
      | [] => (mem, states)
      | bid :: rest =>
        let block := blocks.getD bid Block.empty
        let (mem', flow) := runOps block.ops (states.getD bid (State.fresh 64)) sr widthBits mem
        let visits' := mapIdxN visits (fun i v => if i == bid then v + 1 else v)
        let (states', worklist', visits'') :=
          block.succs.foldl
            (fun (acc : List State × List Nat × List Nat) (tgt : Nat) =>
              let (st, wl, vs) := acc
              let wasSeen := vs.getD tgt 0 > 0
              let st' := if wasSeen
                then mapIdxN st (fun i s => if i == tgt then widenState (st.getD tgt (State.fresh 64)) flow sigOut else s)
                else mapIdxN st (fun i s => if i == tgt then mergeFirstVisit (st.getD tgt (State.fresh 64)) flow else s)
              (st', tgt :: wl, vs))
            (states, rest, visits')
        runCfgLoop blocks sigOut sr widthBits mem' states' worklist' visits'' frest

/-- `verifier::interp::run_cfg` — the worklist fixpoint with back-edge
widening (FR-12). Worklist discipline: LIFO order is not observable in the
interval results (all updates are monotone joins / top-widening), so the
implementation uses head-LIFO; the budget (`nblocks*4+2`) and the
visit/oracle rules mirror the Rust engine exactly. -/
def runCfg (blocks : List Block) (sigIn sigOut : Nat) (sr : Option (Int × Int)) (widthBits : Nat) (mem : MemModel) : MemModel × List State :=
  let nblocks := max blocks.length 1
  let initStates : List State := List.ofFn (fun i : Fin nblocks => if i.1 == 0 then State.calleeEntry sigIn 64 else State.fresh 64)
  let budget0 := nblocks * 4 + 2
  runCfgLoop blocks sigOut sr widthBits mem initStates [0] (List.replicate nblocks 0) (List.range budget0)

/-- `verifier::interp::exit_state` — the join of every ret-block's stepped
state; the returned value sits on top of the abstract stack. The model is
threaded so a ret-block's own memory ops are faithful (P12.2). -/
def exitState (blocks : List Block) (cf : List State) (sr : Option (Int × Int)) (widthBits : Nat) (mem : MemModel) : State :=
  let retBlocks := blocks.filter (fun b => (b.ops.getLast? |>.map (fun o => o.form == Tyu.IR.OpForm.ret)).getD false)
  let (_, acc) := retBlocks.foldl (fun (acc : MemModel × Option State) b =>
    let (m, accSt) := acc
    let (m', flow) := runOps b.ops (cf.getD b.id (State.fresh 1)) sr widthBits m
    (m', some (match accSt with
      | none => flow
      | some a => mergeFirstVisit a flow)))
    (mem, none)
  acc.getD (State.fresh 1)

end Tyu.Conformance
