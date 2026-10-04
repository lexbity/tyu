# labs/ch05/engine-declared-diverge.mod — Lab 5.4 (engine)
# loop is net-zero and never returns: it performs diverge, the word
# declares it, the program runs forever — declared, then observed as HANG.
# Expected: compiles cleanly; hangs; `tyu run --timeout=3` classifies HANG.
module Diverge;
import platform/linux { platform.io.log };

: spin ( -- ) performs {diverge} [ ] loop ;

: main ( -- i64 )
  spin
  "S\n" platform.io.log
  0 ;
export { main };
end;
