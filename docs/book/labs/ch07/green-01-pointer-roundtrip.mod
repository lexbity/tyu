# labs/ch07/green-01-pointer-roundtrip.mod — Lab 7.1 (green)
# A pointer is an ordinary value on the data stack: region-alloc returns
# one, !i64 stores through it, @i64 loads back, and the region is destroyed.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Pointers;
import platform/linux { platform.io.log };
import platform/mem;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  64 as usize platform.mem.region-create
  dup 16 as usize platform.mem.region-alloc
  dup 42 as i64 !i64
  dup @i64 42 == check
  drop
  platform.mem.region-destroy
  "S\n" platform.io.log 0 ;
export { main };
end;
