# labs/ch02/green-04-over-from-locals.mod — Lab 2.4 (green, build-the-vocabulary)
# The checker ships three shuffles (dup swap drop); `over` is missing.
# `over` is written here, in Tyu, out of locals — and tested.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Over;
import platform/linux { platform.io.log };

: check ( bool -- )
  not [ "F\n" platform.io.log ] [ ] if ;

: over ( i64 i64 -- i64 i64 i64 )
  => b => a
  a b a ;

: main ( -- i64 )
  1 2 over + + 4 == check        # 1 2 1  ->  1+2+1 = 4
  7 8 over swap drop == check    # 7 8 7  ->  after swap/drop: 7 7, equal
  "S\n" platform.io.log
  0 ;

export { main };
end;
