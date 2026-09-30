# labs/ch11/red-01-nested-lock.mod — Lab 11.2 (red)
# A lock inside a lock: grants accumulate, forbids accumulate, and the
# nesting itself is rejected. Expected: E5002.
module NestTest;

resource R : i64;

@interrupt(TIMER0) : isr ( -- )
  R lock [ lock [ ] ]
;

: main ( -- i64 ) 0 ;
export { main };
end;
