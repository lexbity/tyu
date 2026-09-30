# labs/ch02/red-01-underflow.mod — Lab 2.5 (red)
# Expected: rejected at compile time — E3202 (stack underflow: nothing to drop).
module Underflow;

: main ( -- i64 )
  drop
  0 ;

export { main };
end;
