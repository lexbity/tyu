# labs/ch02/green-03-while-counter.mod — Lab 2.3 (green)
# Expected: prints "S" (the count-up result equals 10), exits 0.
module CountUp;
import platform/linux { platform.io.log };

: count-up ( i64 -- i64 )
  => limit
  0
  [ dup limit < ] [ 1 + ] while ;

: main ( -- i64 )
  10 count-up 10 == [ "S\n" ] [ "F\n" ] if platform.io.log
  0 ;

export { main };
end;
