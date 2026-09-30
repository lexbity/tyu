# labs/ch04/engine-ensures-violated.mod — Lab 4.3 (engine)
# bump's postcondition is checked after the body: -5 becomes -4, the
# promise about the result breaks, CONTRACT_FAIL.
# Expected: compiles cleanly; traps at run time with CONTRACT_FAIL
# (trap code 20, hosted exit status 20); never emits the marker.
module EnsuresBroken;
import platform/linux { platform.io.log };

: bump ( i64 -- i64 )
  ensures [ dup 0 >= ]
  1 + ;

: main ( -- i64 )
  -5 bump drop
  "S\n" platform.io.log
  0 ;
export { main };
end;
