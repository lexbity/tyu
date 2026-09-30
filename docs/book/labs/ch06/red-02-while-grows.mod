# labs/ch06/red-02-while-grows.mod — Lab 6.3 (red)
# The body pushes one value per lap (dup 1 - has net +1): a loop whose
# stack grows forever, rejected before the first lap.
# Expected: rejected at compile time — E3257 (loop body must be net-zero).
module WhileGrows;

: leaky ( i64 -- )
  [ dup 0 > ] [ dup 1 - ] while drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
