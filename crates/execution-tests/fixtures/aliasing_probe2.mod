module AliasProbe2;
import platform/mem;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

# PINS BY-REF for the region HANDLE too (DEC-15): duplicating a `Region`
# handle yields a second reference to the SAME region, so allocations through
# either handle advance ONE shared bump cursor. Observable: `x` (allocated
# through the original handle) and `y` (allocated through a dup'd handle)
# must NOT overlap — a store at `y` must not clobber the value stored at `x`.
# (Copied-region semantics would give them the same base and the later store
# would overwrite.)
: aliasing-probe2 ( -- )
  64 as usize platform.mem.region-create
  dup 16 as usize platform.mem.region-alloc
  dup 42 as i64 !i64
  swap dup 8 as usize platform.mem.region-alloc
  7 as i64 !i64
  drop
  @i64 42 == check
;

export { aliasing-probe2 };
end;