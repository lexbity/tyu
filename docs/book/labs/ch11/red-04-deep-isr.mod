# labs/ch11/red-04-deep-isr.mod — Lab 11.5 (red, inspection check)
# The ISR's own stack has a budget; 34 plates at peak exceeds it. Check
# with: langc --emit=ir red-04-deep-isr.mod --sysroot=<sysroot>
# Expected: rejected — E5030 (ISR stack exceeds its budget).
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
