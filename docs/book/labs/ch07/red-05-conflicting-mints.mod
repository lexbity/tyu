# labs/ch07/red-05-conflicting-mints.mod — Lab 7.8 (red)
# Two mutable borrows of the same root alive at once: one writer or many
# readers, never both. Expected: rejected at compile time — E5021.
module ConflictingMints;

resource Counter : i64;

: two-mints ( -- )
  Counter lock [ &!Counter &!Counter drop drop ] ;

: main ( -- i64 ) 0 ;
export { main };
end;
