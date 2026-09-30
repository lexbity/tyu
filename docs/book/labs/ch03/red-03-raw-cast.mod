# labs/ch03/red-03-raw-cast.mod — Lab 3.7 (red)
# Raw pointer casts are compile-time errors — with or without
# langc --allow-raw-casts on the current tree. Expected: E3305.
module RawCast;

: bad ( i64 -- )
  as ptr drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
