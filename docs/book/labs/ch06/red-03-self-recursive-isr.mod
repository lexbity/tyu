# labs/ch06/red-03-self-recursive-isr.mod — Lab 6.4 (red)
# An ISR must have a finite stack bound: a self-recursive handler is
# unbounded (high = top) and is rejected at the binding site.
# Expected: rejected at compile time — E5040.
module RecursiveIsr;

@interrupt(TIMER0) : isr ( -- )
  isr
;

: main ( -- i64 ) 0 ;
export { main };
end;
