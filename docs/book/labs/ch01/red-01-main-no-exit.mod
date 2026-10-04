# labs/ch01/red-01-main-no-exit.mod — Lab 1.4 (red)
# Expected: rejected at compile time — E1018 (main must return the exit code).
module NoExit;
import platform/linux { platform.io.log };

: main ( -- )
  "S\n" platform.io.log ;

export { main };
end;
