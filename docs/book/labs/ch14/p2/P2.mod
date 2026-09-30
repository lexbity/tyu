# labs/ch14/p2/P2.mod — Lab 14.1 (green)
# One subtype in parameter position: the pipeline renders its range
# promise as a statement for the developer to prove.
# Expected: builds through the Lean pipeline; one statement rendered.
module P2;
subtype Percent = i64 range 0 .. 100;
: take ( Percent -- ) drop ;
: main ( -- i64 ) 50 as Percent take 0 ;
export { main };
end;
