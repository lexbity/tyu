# expect: E5001
module Main;
@interrupt(TIMER0) : isr_yield ( -- )
  platform.task.yield
;
end;
