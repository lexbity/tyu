# labs/ch03/red-01-enum-distinct.mod — Lab 3.5 (red)
# A State is not its number: passing the variant where an i64 is declared
# is a compile-time type mismatch. Expected: E3212.
module EnumDistinct;

enum State : u8
  Idle = 0x00
  Run  = 0x01
end;

: takes-int ( i64 -- ) drop ;

: main ( -- i64 )
  State.Run takes-int
  0 ;
export { main };
end;
