# labs/ch07/red-03-move-twice.mod — Lab 7.6 (red)
# x was moved once (pushed as x); pushing it again is use after move.
# Expected: rejected at compile time — E5012.
module MoveTwice;

iso Msg;

: move-twice ( Msg -- Msg )
  => x
  x x ;

: main ( -- i64 ) 0 ;
export { main };
end;
