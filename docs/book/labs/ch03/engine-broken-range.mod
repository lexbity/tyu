# labs/ch03/engine-broken-range.mod — Lab 3.4 (engine)
# Expected: compiles cleanly; traps at run time with SUBTYPE_FAIL
# (trap code 21, hosted exit status 21); never emits the marker.
module BrokenRange;
import platform/linux { platform.io.log };

subtype Percent = i64 range 0 .. 100;

: main ( -- i64 )
  150 as Percent drop   # the lie: 150 is not a Percent
  "S\n" platform.io.log
  0 ;
export { main };
end;
