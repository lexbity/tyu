# labs/ch05/red-04-suspend-under-lock.mod — Lab 5.8 (red)
# A lock forbids suspend: yielding while holding a device is exactly the
# deadlock the rule exists to prevent. Expected: E5001.
module SuspendUnderLock;
import platform/linux { platform.task.yield platform.task.run };

resource Counter : i64;

: poll-under-lock ( -- )
  Counter lock [ [ platform.task.yield ] platform.task.run ] ;

: main ( -- i64 ) 0 ;
export { main };
end;
