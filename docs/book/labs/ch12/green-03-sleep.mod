# labs/ch12/green-03-sleep.mod — Lab 12.3 (green)
# sleep belongs to the same suspend family as yield: it performs
# {suspend} and runs inside the run handler that discharges it.
# Expected: prints napping, then awake, then the marker; exits 0.
module Nap;
import platform/linux { platform.io.log platform.task.run platform.task.sleep-ms };

: main ( -- i64 )
  [ ( -- ) "napping\n" platform.io.log 1 as usize platform.task.sleep-ms "awake\n" platform.io.log ] platform.task.run
  "S\n" platform.io.log 0 ;
export { main };
end;
