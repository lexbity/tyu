# labs/ch04/green-02-ensures.mod — Lab 4.2 (green)
# An ensures clause is checked after the body, against the outputs:
# bump promises its RESULT is zero or more.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Bump;
import platform/linux { platform.io.log };

: bump ( i64 -- i64 )
  ensures [ dup 0 >= ]
  1 + ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  5 bump 6 == check
  0 bump 1 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
