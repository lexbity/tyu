module platform/linux;
type Task;

: platform.io.log ( str -- )
  drop
;

: platform.time.now_ms ( -- i64 )
  0
;

: platform.critical.enter ( -- )
;

: platform.critical.exit ( -- )
;

: platform.task.run ( quot -- )
  # compiler handler: runs the quotation with suspend allowed
;

: platform.task.spawn ( quot -- Task )
  # hosted: runtime-provided task spawn
;

: platform.task.join ( Task -- ) performs {suspend}
  # hosted: runtime-provided task join
;

: platform.task.yield ( -- ) performs {suspend}
  # hosted: `sched_yield` syscall (cooperative yield)
;

: platform.task.sleep-ms ( usize -- ) performs {suspend}
  # hosted: runtime-provided sleep (milliseconds)
;

: platform.task.sleep-us ( usize -- ) performs {suspend}
  # hosted: runtime-provided sleep (microseconds)
;

end;
