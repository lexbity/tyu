# labs/ch05/green-01-alloc-effect.mod — Lab 5.1 (green)
# grab allocates from a region and says so: performs {alloc} is the label
# on the tin, checked into the word's interface. The region is created,
# used for one typed store/load round-trip, and destroyed.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Grab;
import platform/linux { platform.io.log };
import platform/mem;

: grab ( -- )
  performs {alloc}
  64 as usize platform.mem.region-create
  dup 16 as usize platform.mem.region-alloc
  dup 42 as i64 !i64
  dup @i64 42 == check
  drop
  platform.mem.region-destroy ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  grab
  "S\n" platform.io.log 0 ;
export { main };
end;
