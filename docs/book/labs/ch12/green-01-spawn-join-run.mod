# labs/ch12/green-01-spawn-join-run.mod — Lab 12.1 (green)
# The full pattern: spawn a worker (its quotation escapes, so it carries
# an annotation), join it, and only then check the shared resource.
# join performs suspend, so the pair lives inside a run handler.
# Expected: silent clean run (exit 0, marker emitted, no F).
module SpawnJoin;
import platform/linux { platform.io.log platform.task.spawn platform.task.join platform.task.run };

resource Counter : i64;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  [ ( -- )
    [ ( -- ) Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ] ] platform.task.spawn
    platform.task.join
  ] platform.task.run
  Counter lock [ &!Counter @i64 1 == check ]
  "S\n" platform.io.log 0 ;
export { main };
end;
