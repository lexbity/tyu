# labs/ch05/red-01-undeclared-effect.mod — Lab 5.5 (red)
# loop performs diverge; performs {} declares none of it.
# Expected: rejected at compile time — E5005 (undeclared effect).
module Undeclared;

: forever ( -- ) performs {} [ ] loop ;

: main ( -- i64 ) 0 ;
export { main };
end;
