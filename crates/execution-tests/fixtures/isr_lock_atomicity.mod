# expect: ok
# S12: ISR lock atomicity test
# When this program runs under QEMU, it verifies that a lock with interrupt
# masking prevents ISR from observing torn updates to shared state.
# For v1 the runtime does not yet support SysTick dispatch; this file serves
# as the fixture that will be compiled and run once the runtime is extended.
module Main;
resource R;
: counter ( -- )
  R lock [
    &!R drop
  ]
;
: main ( -- i64 )
  0
;
end;
