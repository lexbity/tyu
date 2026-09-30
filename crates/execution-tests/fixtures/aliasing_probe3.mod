module AliasProbe3;
import platform/mem;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

# PINS the 8-byte allocation ALIGNMENT of `region-alloc` (DEC-15 §Q11):
# native rounds every allocation up to a multiple of 8 (`add 7; and -8`),
# so allocating 1 byte still advances the region cursor by 8. Observable:
# a second 1-byte allocation lands 8 bytes away and cannot clobber the
# first — if alignment were 1, the second store would overwrite the first
# value and the check would fail.
: aliasing-probe3 ( -- )
  64 as usize platform.mem.region-create
  dup 1 as usize platform.mem.region-alloc
  dup 5 as i64 !i64
  swap dup 1 as usize platform.mem.region-alloc
  6 as i64 !i64
  drop
  @i64 5 == check
;

export { aliasing-probe3 };
end;