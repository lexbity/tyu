# labs/ch01/red-02-stack-lie.mod — Lab 1.5 (red)
# Expected: rejected at compile time — E3220 (declared stack effect vs body).
module StackLie;

: answer ( -- i64 ) ;

: main ( -- i64 )
  answer
  "S\n" platform.io.log
  0 ;

import platform/linux { platform.io.log };
export { main };
end;
