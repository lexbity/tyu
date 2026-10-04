# labs/ch09/green-02-reused-loop-bodies.mod — Lab 9.2 (green)
# One net-zero body, two provable loops: tick is factored once and both
# whiles inherit its shape for free.
# Expected: silent clean run (exit 0, marker emitted, no F).
module ReusedBodies;
import platform/linux { platform.io.log };

: tick ( i64 -- i64 ) 1 - ;

: two-loops ( i64 i64 -- i64 i64 )
  => b => a
  a [ dup 0 > ] [ tick ] while
  b [ dup 10 > ] [ tick ] while ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  3 12 two-loops
  => y => x
  x 0 == check
  y 10 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
