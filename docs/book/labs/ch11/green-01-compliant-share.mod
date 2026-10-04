# labs/ch11/green-01-compliant-share.mod — Lab 11.1 (green)
# The compliant shape: the ISR and mainline both touch Counter, both
# inside their own lock. The cross-context rule is satisfied, and the
# lock lowers to whatever the platform needs (hosted: a fence).
# Expected: builds clean on hosted. The image itself is not runnable:
# an ISR binding is metal (chapter 8's QEMU track and real boards) - the
# compile-time binding checks are the lab.
module Compliant;
import platform/linux { platform.io.log };

resource Counter : i64;

@interrupt(TIMER0) : isr ( -- )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
;

: main ( -- i64 )
  Counter lock [ &!Counter @i64 drop ]
  "S\n" platform.io.log 0 ;
export { main };
end;
