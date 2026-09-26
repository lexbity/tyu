namespace Tyu.IR

/- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by
`crates/verifier/src/export/lean.rs` — DO NOT EDIT. Drift-locked:
`TYU_EXPORT_PORTS=1 cargo test -p verifier --test export_drift`
regenerates; the committed file must byte-match. -/
/-- One printable op form: the canonical `--emit=ir` mnemonic (one
constructor per row of `verifier::semantics`, 42 rows). -/
inductive OpForm where
  | const_i64
  | const_bool
  | const_str
  | addr_of
  | addr_of_mut
  | mmio_place
  | scoped_enter
  | task_spawn
  | ptr_add_const
  | ptr_add_index
  | dup
  | drop
  | swap
  | add_i64
  | sub_i64
  | mul_i64
  | cmp_lt
  | cmp_le
  | cmp_gt
  | cmp_ge
  | cmp_eq
  | cmp_ne
  | and_bool
  | or_bool
  | not_bool
  | interrupt_disable
  | interrupt_enable
  | local_set
  | local_get
  | cast
  | bitcast
  | call
  | load
  | store
  | vol_load
  | vol_store
  | vol_load_field
  | vol_store_field
  | trap_if_false
  | br
  | br_if
  | ret
  deriving DecidableEq, Repr, Inhabited

/-- The canonical `--emit=ir` mnemonic of each op form (the `mnemonic`
column of the semantics table; the op-text parser dispatches on it). -/
def OpForm.mnemonic : OpForm → String
  | .const_i64 => "const_i64"
  | .const_bool => "const_bool"
  | .const_str => "const_str"
  | .addr_of => "addr_of"
  | .addr_of_mut => "addr_of_mut"
  | .mmio_place => "mmio_place"
  | .scoped_enter => "scoped_enter"
  | .task_spawn => "task_spawn"
  | .ptr_add_const => "ptr_add_const"
  | .ptr_add_index => "ptr_add_index"
  | .dup => "dup"
  | .drop => "drop"
  | .swap => "swap"
  | .add_i64 => "add_i64"
  | .sub_i64 => "sub_i64"
  | .mul_i64 => "mul_i64"
  | .cmp_lt => "cmp_lt"
  | .cmp_le => "cmp_le"
  | .cmp_gt => "cmp_gt"
  | .cmp_ge => "cmp_ge"
  | .cmp_eq => "cmp_eq"
  | .cmp_ne => "cmp_ne"
  | .and_bool => "and_bool"
  | .or_bool => "or_bool"
  | .not_bool => "not_bool"
  | .interrupt_disable => "interrupt_disable"
  | .interrupt_enable => "interrupt_enable"
  | .local_set => "local_set"
  | .local_get => "local_get"
  | .cast => "cast"
  | .bitcast => "bitcast"
  | .call => "call"
  | .load => "load"
  | .store => "store"
  | .vol_load => "vol_load"
  | .vol_store => "vol_store"
  | .vol_load_field => "vol_load_field"
  | .vol_store_field => "vol_store_field"
  | .trap_if_false => "trap_if_false"
  | .br => "br"
  | .br_if => "br_if"
  | .ret => "ret"

end Tyu.IR
