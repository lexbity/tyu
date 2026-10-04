# labs/ch05/red-03-suspend-in-main.mod — Lab 5.7 (red)
# yield performs suspend; main is not a suspendable context and nothing
# here discharges it. Expected: rejected at compile time — E5001.
module SuspendInMain;
import platform/linux { platform.task.yield };

: main ( -- i64 )
  platform.task.yield
  0 ;
export { main };
end;
