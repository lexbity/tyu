# labs/ch04/red-01-flat-sandwich.mod — Lab 4.4 (red)
# This predicate READS like a two-sided range check, but its stack math is
# flat: dup+1, 0+1, >=-1, swap, 100+1, <=-1, and-1 = net 0, no bool left.
# Expected: rejected at compile time — E3310 (predicate must end with
# exactly one bool on top).
module FlatSandwich;

: set-range ( i64 -- i64 )
  needs [ dup 0 >= swap 100 <= and ]
  as i64 ;

: main ( -- i64 ) 50 set-range drop 0 ;
export { main };
end;
