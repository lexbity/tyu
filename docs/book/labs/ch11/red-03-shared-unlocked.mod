# labs/ch11/red-03-shared-unlocked.mod — Lab 11.4 (red)
# The cross-context rule: Counter is reachable from the ISR, so every
# access anywhere must be inside a lock - main's unlocked borrow is the
# race the rule exists to prevent. Expected: E5031.
module SharedUnlocked;

resource Counter : i64;

@interrupt(TIMER0) : isr ( -- )
  Counter lock [ &!Counter @i64 1 + &!Counter swap !i64 ]
;

: main ( -- i64 )
  &!Counter @i64 drop
  0 ;
export { main };
end;
