# labs/ch02/green-01-arithmetic-check.mod — Lab 2.1 (green)
# Expected: silent clean run (exit 0, marker emitted, no F).
module ArithmeticCheck;
import platform/linux { platform.io.log };

: check ( bool -- )
  not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  2 3 + 5 == check
  10 3 - 7 == check
  4 5 * 20 == check
  3 5 < check
  7 2 > check
  "S\n" platform.io.log
  0 ;

export { main };
end;
