# labs/ch12/green-02-two-workers.mod — Lab 12.2 (green)
# Two workers increment the same counter; both are joined before the
# check. The result is deterministic even though the execution is not.
# Expected: silent clean run (exit 0, marker emitted, no F).
module TwoWorkers;
import platform/linux { platform.io.log platform.task.spawn platform.task.join platform.task.run };

resource Counter : i64;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  [ ( -- )
    [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn platform.task.join
    [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn platform.task.join
  ] platform.task.run
  Counter lock [ &!Counter @i64 2 == check ]
  "S\n" platform.io.log 0 ;
export { main };
end;
