# labs/ch01/red-03-unknown-word.mod — Lab 1.6 (red)
# Expected: rejected at compile time — E3210 (WordNotFound).
module Unknown;

: warmth ( i64 -- i64 )
  1 + ;

: main ( -- i64 )
  41 OOPS
  warmth
  "S\n" platform.io.log
  0 ;

import platform/linux { platform.io.log };
export { main };
end;
