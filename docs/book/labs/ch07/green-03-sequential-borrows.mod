# labs/ch07/green-03-sequential-borrows.mod — Lab 7.3 (green)
# One borrow at a time is always fine: two sequential borrow blocks over
# the same array, each closed before the next opens.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Sequential;
import platform/linux { platform.io.log };

: reborrow ( i64'4 -- i64'4 )
  &[ drop ] &[ drop ] ;

: main ( -- i64 )
  "S\n" platform.io.log 0 ;
export { main };
end;
