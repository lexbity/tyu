# expect: E5040
module Main;
@interrupt(TIMER0) : isr ( -- )
  isr
;
end;
