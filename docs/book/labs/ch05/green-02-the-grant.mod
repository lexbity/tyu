# labs/ch05/green-02-the-grant.mod — Lab 5.2 (green)
# The resource is writable only inside its lock: the lock is what grants
# write(Counter), and the grant covers the code lexically inside.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Counter;
import platform/linux { platform.io.log };

resource Counter : i64;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
  Counter lock [ &!Counter @i64 1 == check ]
  "S\n" platform.io.log 0 ;
export { main };
end;
