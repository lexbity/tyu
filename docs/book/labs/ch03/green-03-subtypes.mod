# labs/ch03/green-03-subtypes.mod — Lab 3.3 (green)
# A subtype carries its range in the type. Words exchange the BASE type;
# `as Percent` converts (and range-traps), `as? Percent` asks politely.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Subtypes;
import platform/linux { platform.io.log };

subtype Percent = i64 range 0 .. 100;

: take ( Percent -- ) drop ;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  42 as Percent take               # convert at the call site
  42 as Percent as i64 42 == check # round-trip through the subtype
  7 as? Percent swap drop check    # in range: ok = true
  150 as? Percent swap drop not check  # out of range: ok = false
  "S\n" platform.io.log 0 ;
export { main };
end;
