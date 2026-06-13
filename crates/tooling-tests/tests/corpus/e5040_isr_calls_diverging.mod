# expect: E5040
module Main;
: diverter ( -- ) performs {diverge}
  [ ] loop
;
@interrupt(TIMER0) : isr ( -- )
  diverter
;
end;
