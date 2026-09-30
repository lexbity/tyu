# labs/ch06/red-01-accumulating-shape.mod — Lab 6.2 (red)
# Each pass keeps a copy alive below the recursive call: the branches have
# net +1 against the declared net 0. The shape rules refuse the growth
# before it exists. Expected: rejected at compile time — E3220.
module Accumulating;

: boom ( i64 -- i64 )
  dup 0 >= [ dup 1 - boom ] [ dup ] if ;

: main ( -- i64 ) 0 ;
export { main };
end;
