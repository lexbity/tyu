# labs/ch09/green-01-factored-contract.mod — Lab 9.1 (green)
# The two-sided range contract, factored: bounded-by keeps the value and
# returns value+verdict — the shape the contract rules demand from a
# helper. The needs clause reads as a sentence and still traps.
# Expected: silent clean run (exit 0, marker emitted, no F).
module FactoredRange;
import platform/linux { platform.io.log };

: bounded-by ( i64 i64 -- i64 bool )
  => hi dup hi <= ;

: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ 100 bounded-by ] [ false ] if ]
  as i64 ;

: main ( -- i64 )
  0 set-range drop
  50 set-range drop
  100 set-range drop
  "S\n" platform.io.log 0 ;
export { main };
end;
