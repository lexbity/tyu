# labs/ch07/red-01-iso-dup.mod — Lab 7.4 (red)
# An iso value is custody; duplicating it would be two owners of one thing.
# Expected: rejected at compile time — E5010.
module IsoDup;

iso Msg;

: bad-dup ( Msg -- Msg Msg ) dup ;

: main ( -- i64 ) 0 ;
export { main };
end;
