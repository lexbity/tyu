# labs/ch07/red-04-borrow-escape.mod — Lab 7.7 (red)
# The borrow lives exactly inside its block; carrying it out (return) is
# an escape. Expected: rejected at compile time — E5020.
module BorrowEscape;

: escape-return ( i64'1 -- Slice(i64) )
  &[
    swap drop
    return
  ] ;

: main ( -- i64 ) 0 ;
export { main };
end;
