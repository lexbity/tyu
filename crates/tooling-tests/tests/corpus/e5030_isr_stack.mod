# expect: E5030
module Main;
@interrupt(TIMER0) : isr ( -- )
  0
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup dup dup dup dup dup dup dup
  dup dup dup
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop
;
end;
