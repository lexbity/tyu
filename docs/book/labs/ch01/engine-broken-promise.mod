# labs/ch01/engine-broken-promise.mod — Lab 1.3 (engine)
# Expected: compiles cleanly; traps at run time with CONTRACT_FAIL
# (trap code 20, hosted exit status 20); never emits the marker.
module BrokenPromise;
import platform/linux { platform.io.log };

: set-level ( i64 -- )
  needs [ dup 0 >= ]
  => level
  "level accepted\n" platform.io.log ;

: main ( -- i64 )
  42 set-level
  -5 set-level
  "S\n" platform.io.log
  0 ;

export { main };
end;
