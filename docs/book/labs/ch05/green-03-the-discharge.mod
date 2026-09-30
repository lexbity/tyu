# labs/ch05/green-03-the-discharge.mod — Lab 5.3 (green)
# platform.task.run is the handler that discharges suspend: inside its
# quotation the task may yield; when it finishes, main continues.
# Expected: prints "ticked", then the marker; exits 0.
module Discharge;
import platform/linux { platform.io.log };

: main ( -- i64 )
  [ "ticked\n" platform.io.log platform.task.yield platform.task.yield ] platform.task.run
  "S\n" platform.io.log 0 ;
export { main };
end;
