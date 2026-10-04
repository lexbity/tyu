# labs/ch12/red-02-join-outside-handler.mod — Lab 12.5 (red)
# join performs suspend; main is not a suspendable context, so the join
# has nothing to discharge it. Expected: E5001.
module JoinOutside;
import platform/linux { platform.task.spawn platform.task.join };

resource Counter : i64;

: main ( -- i64 )
  [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn
  platform.task.join
  0 ;
export { main };
end;
