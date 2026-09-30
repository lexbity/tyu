module AliasProbe1;
import platform/mem;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

# PINS BY-REF semantics (PLAN-WASM-1 DEC-15 / §Q11):
# `dup` of a region-alloc'd pointer copies the REFERENCE, not the aggregate.
# Native x86_64 duplicates a single 8-byte slot, so two references from a
# `dup` alias the SAME storage: a store through one is visible through the
# other. (A by-value eager-copy engine would diverge here by construction —
# this fixture exists to kill that divergence class via G53.)
: aliasing-probe1 ( -- )
  64 as usize platform.mem.region-create
  dup 8 as usize platform.mem.region-alloc
  dup 7 as i64 !i64
  @i64 7 == check
  drop
;

export { aliasing-probe1 };
end;