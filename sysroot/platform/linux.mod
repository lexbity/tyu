module platform/linux;
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

: platform.task.yield ( -- ) !{suspend}
  # hosted: `sched_yield` syscall (cooperative yield)
;

end;
