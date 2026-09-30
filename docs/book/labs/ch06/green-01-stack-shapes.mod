# labs/ch06/green-01-stack-shapes.mod — Lab 6.1 (green)
# Three shapes and one recursion: a tall peak, symmetric branches, a loop's
# temporary, and a net-zero countdown — each with a bounded (net, high).
# Expected: silent clean run (exit 0, marker emitted, no F).
module Shapes;
import platform/linux { platform.io.log };

: peak-10 ( -- )
  1 2 3 4 5 6 7 8 9 10
  drop drop drop drop drop
  drop drop drop drop drop ;

: symmetric-branch ( -- )
  true
  [ 1 2 3 4 5 drop drop drop drop drop ]
  [ 6 7 8 9 10 drop drop drop drop drop ]
  if ;

: loop-temp ( -- )
  1 [ dup 0 > ] [ 1 - ] while drop ;

: countdown ( i64 -- i64 )
  dup 0 <= [ ] [ 1 - countdown ] if ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  peak-10
  symmetric-branch
  loop-temp
  1000 countdown 0 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
