# labs/ch12/red-01-unannotated-quotation.mod — Lab 12.4 (red)
# spawn's quotation escapes into the task system: it must carry its
# stack-effect annotation. Expected: rejected at compile time — E3760.
module Unannotated;
import platform/linux { platform.task.spawn platform.task.join platform.task.run };

: main ( -- i64 )
  [ ( -- )
    [ 1 2 + ] platform.task.spawn
    platform.task.join
  ] platform.task.run
  0 ;
export { main };
end;
