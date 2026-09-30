# labs/ch08/red-03-misaligned.mod — Lab 8.5 (red)
# A 32-bit register at offset 0x01: hardware would tolerate it; the
# checker will not. Expected: rejected at compile time — E3611.
module Misaligned;

register-map Bad
  0x01 A u32 rw volatile
end;

const bad = Bad @ board.scratch ;

: main ( -- i64 ) 0 ;
export { main };
end;
