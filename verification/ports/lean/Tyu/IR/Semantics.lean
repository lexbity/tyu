import Tyu.IR.Op
import Std

namespace Tyu.IR

/- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by
`crates/verifier/src/export/lean.rs` — DO NOT EDIT. Mirrors
`src/semantics/ops.json` (tyu.ir-sem/1.0). -/
/-- One row of the normative semantics table (`verifier::semantics::SemanticsRow`). -/
structure SemanticsRow where
form : OpForm
mnemonic : String
pops : Nat
pushes : Nat
effect_bits : Nat
oel : String
deriving DecidableEq, Repr

/-- The table, in IR text-printer order. -/
def rows : List SemanticsRow := [
  { form := .const_i64, mnemonic := "const_i64", pops := 0, pushes := 1, effect_bits := 0, oel := "value:const" },
  { form := .const_bool, mnemonic := "const_bool", pops := 0, pushes := 1, effect_bits := 0, oel := "value:const" },
  { form := .const_str, mnemonic := "const_str", pops := 0, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .addr_of, mnemonic := "addr_of", pops := 0, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .addr_of_mut, mnemonic := "addr_of_mut", pops := 0, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .mmio_place, mnemonic := "mmio_place", pops := 0, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .scoped_enter, mnemonic := "scoped_enter", pops := 0, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .task_spawn, mnemonic := "task_spawn", pops := 0, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .ptr_add_const, mnemonic := "ptr_add_const", pops := 1, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .ptr_add_index, mnemonic := "ptr_add_index", pops := 2, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .dup, mnemonic := "dup", pops := 1, pushes := 2, effect_bits := 0, oel := "value:dup" },
  { form := .drop, mnemonic := "drop", pops := 1, pushes := 0, effect_bits := 0, oel := "value:drop" },
  { form := .swap, mnemonic := "swap", pops := 2, pushes := 2, effect_bits := 0, oel := "value:swap" },
  { form := .add_i64, mnemonic := "add_i64", pops := 2, pushes := 1, effect_bits := 0, oel := "value:add" },
  { form := .sub_i64, mnemonic := "sub_i64", pops := 2, pushes := 1, effect_bits := 0, oel := "value:sub" },
  { form := .mul_i64, mnemonic := "mul_i64", pops := 2, pushes := 1, effect_bits := 0, oel := "value:mul" },
  { form := .cmp_lt, mnemonic := "cmp_lt", pops := 2, pushes := 1, effect_bits := 0, oel := "value:cmp_lt" },
  { form := .cmp_le, mnemonic := "cmp_le", pops := 2, pushes := 1, effect_bits := 0, oel := "value:cmp_le" },
  { form := .cmp_gt, mnemonic := "cmp_gt", pops := 2, pushes := 1, effect_bits := 0, oel := "value:cmp_gt" },
  { form := .cmp_ge, mnemonic := "cmp_ge", pops := 2, pushes := 1, effect_bits := 0, oel := "value:cmp_ge" },
  { form := .cmp_eq, mnemonic := "cmp_eq", pops := 2, pushes := 1, effect_bits := 0, oel := "value:cmp_eq" },
  { form := .cmp_ne, mnemonic := "cmp_ne", pops := 2, pushes := 1, effect_bits := 0, oel := "value:cmp_ne" },
  { form := .and_bool, mnemonic := "and_bool", pops := 2, pushes := 1, effect_bits := 0, oel := "value:and" },
  { form := .or_bool, mnemonic := "or_bool", pops := 2, pushes := 1, effect_bits := 0, oel := "value:or" },
  { form := .not_bool, mnemonic := "not_bool", pops := 1, pushes := 1, effect_bits := 0, oel := "value:not" },
  { form := .interrupt_disable, mnemonic := "interrupt_disable", pops := 0, pushes := 0, effect_bits := 0, oel := "opaque" },
  { form := .interrupt_enable, mnemonic := "interrupt_enable", pops := 0, pushes := 0, effect_bits := 0, oel := "opaque" },
  { form := .local_set, mnemonic := "local_set", pops := 1, pushes := 0, effect_bits := 0, oel := "value:local_set" },
  { form := .local_get, mnemonic := "local_get", pops := 0, pushes := 1, effect_bits := 0, oel := "value:local_get" },
  { form := .cast, mnemonic := "cast", pops := 1, pushes := 1, effect_bits := 0, oel := "value:cast" },
  { form := .bitcast, mnemonic := "bitcast", pops := 1, pushes := 1, effect_bits := 0, oel := "value:bitcast" },
  { form := .call, mnemonic := "call", pops := 1, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .load, mnemonic := "load", pops := 1, pushes := 1, effect_bits := 0, oel := "opaque" },
  { form := .store, mnemonic := "store", pops := 2, pushes := 0, effect_bits := 0, oel := "opaque" },
  { form := .vol_load, mnemonic := "vol_load", pops := 1, pushes := 1, effect_bits := 8, oel := "opaque" },
  { form := .vol_store, mnemonic := "vol_store", pops := 2, pushes := 0, effect_bits := 8, oel := "opaque" },
  { form := .vol_load_field, mnemonic := "vol_load_field", pops := 1, pushes := 1, effect_bits := 8, oel := "opaque" },
  { form := .vol_store_field, mnemonic := "vol_store_field", pops := 2, pushes := 0, effect_bits := 8, oel := "opaque" },
  { form := .trap_if_false, mnemonic := "trap_if_false", pops := 1, pushes := 0, effect_bits := 0, oel := "control" },
  { form := .br, mnemonic := "br", pops := 0, pushes := 0, effect_bits := 0, oel := "control" },
  { form := .br_if, mnemonic := "br_if", pops := 1, pushes := 0, effect_bits := 0, oel := "control" },
  { form := .ret, mnemonic := "ret", pops := 0, pushes := 0, effect_bits := 0, oel := "control" }
]

theorem row_count : rows.length = 42 := by
  decide

/-- Completeness: every op form has a row in the table (P3.1's
`SEMANTICS_total`; kernel-checked by `decide` — a hand-edited generated
file dropping a row fails the port build). -/
theorem SEMANTICS_total : ∀ f : OpForm, ∃ r : SemanticsRow, r ∈ rows ∧ r.form = f := by
  intro f
  cases f with
    | const_i64 => decide
    | const_bool => decide
    | const_str => decide
    | addr_of => decide
    | addr_of_mut => decide
    | mmio_place => decide
    | scoped_enter => decide
    | task_spawn => decide
    | ptr_add_const => decide
    | ptr_add_index => decide
    | dup => decide
    | drop => decide
    | swap => decide
    | add_i64 => decide
    | sub_i64 => decide
    | mul_i64 => decide
    | cmp_lt => decide
    | cmp_le => decide
    | cmp_gt => decide
    | cmp_ge => decide
    | cmp_eq => decide
    | cmp_ne => decide
    | and_bool => decide
    | or_bool => decide
    | not_bool => decide
    | interrupt_disable => decide
    | interrupt_enable => decide
    | local_set => decide
    | local_get => decide
    | cast => decide
    | bitcast => decide
    | call => decide
    | load => decide
    | store => decide
    | vol_load => decide
    | vol_store => decide
    | vol_load_field => decide
    | vol_store_field => decide
    | trap_if_false => decide
    | br => decide
    | br_if => decide
    | ret => decide

end Tyu.IR
