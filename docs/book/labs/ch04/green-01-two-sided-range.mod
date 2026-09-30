# labs/ch04/green-01-two-sided-range.mod — Lab 4.1 (green)
# The two-sided range contract in one needs clause: the first comparison
# guards, the if combines, the input survives, one bool remains.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Range;
import platform/linux { platform.io.log };

: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ]
  as i64 ;

: main ( -- i64 )
  0 set-range drop
  50 set-range drop
  100 set-range drop
  "S\n" platform.io.log 0 ;
export { main };
end;
