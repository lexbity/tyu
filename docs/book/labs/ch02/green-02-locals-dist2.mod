# labs/ch02/green-02-locals-dist2.mod — Lab 2.2 (green)
# Expected: silent clean run (exit 0, marker emitted, no F).
module Dist2;
import platform/linux { platform.io.log };

: check ( bool -- )
  not [ "F\n" platform.io.log ] [ ] if ;

: dist2 ( i64 i64 i64 i64 -- i64 )
  => y2 => x2 => y1 => x1
  x2 x1 - dup *
  y2 y1 - dup * + ;

: main ( -- i64 )
  1 2 4 6 dist2 25 == check
  "S\n" platform.io.log
  0 ;

export { main };
end;
