# labs/ch04/red-02-predicate-not-bool.mod — Lab 4.5 (red)
# The predicate ends one value on top — but it is an i64, not a bool.
# Expected: rejected at compile time — E3311 (contract not bool).
module NotBool;

: f ( i64 -- )
  needs [ dup ]
  drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
