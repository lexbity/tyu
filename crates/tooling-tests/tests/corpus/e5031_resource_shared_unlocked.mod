# expect: E5031
module Main;
resource R;
@interrupt(TIMER0) : isr ( -- )
  &!R drop
;
: main ( -- )
  &!R drop
;
end;
