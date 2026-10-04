# labs/ch11/red-02-isr-yield.mod — Lab 11.3 (red)
# An ISR forbids suspend: there is no scheduler to yield to from an
# interrupt. Expected: rejected at compile time — E5001.
module IsrYield;
import platform/linux { platform.task.yield };

@interrupt(TIMER0) : isr ( -- )
  platform.task.yield
;

: main ( -- i64 ) 0 ;
export { main };
end;
