# labs/ch07/red-02-iso-drop.mod — Lab 7.5 (red)
# An iso value cannot be silently discarded either: dropping one needs an
# explicit destructor capability. Expected: E5011.
module IsoDrop;

iso Msg;

: bad-drop ( Msg -- ) drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
