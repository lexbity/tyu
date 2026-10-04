# labs/ch04/red-03-ensures-shape.mod — Lab 4.6 (red)
# Postconditions follow the same shape rules: this one leaves nothing
# (net 0). Expected: rejected at compile time — E3310.
module EnsuresShape;

: f ( i64 -- i64 )
  ensures [ 0 >= ]
  1 + ;

: main ( -- i64 ) 0 ;
export { main };
end;
