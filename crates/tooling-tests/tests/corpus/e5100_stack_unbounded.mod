# expect: E5100
module Main;
@interrupt(TIMER0) : isr ( -- )
  0 drop
;
end;
