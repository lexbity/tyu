module IsrLockAtomicity;
import platform/testio { testio.write-byte };

resource Counter : i64 = 0 ceiling 1;

register-map SysTick
  0x00 CTRL u32 rw volatile
  0x04 LOAD u32 rw volatile
  0x08 VAL u32 rw volatile
end;

# ISR re-entry guard cell: SysTick pends on every wrap while TICKINT stays
# set, and wraps landing between handler entry and the first store re-latch
# the pending bit (the NVIC tail-chains one extra entry; SW pending clears
# are not effective for SysTick). The guard is an MMIO scratch cell, not a
# resource: it is touched only from ISR context, where same-priority
# exceptions never nest (a tail-chain runs strictly after the previous entry
# returned). First entry stops the timer, flips the guard, and does the
# Counter update; any tail-chained re-entry sees A != 0 and returns, so
# exactly one ISR increment reaches `Counter` per run.
register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;

const systick = SysTick @ board.systick;
const scratch = Scratch @ board.scratch;

: burn ( -- )
  0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop
  0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop
  0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop ;

@interrupt(SysTick)
: on_tick ( -- )
  &!systick.CTRL 0 as u32 !u32
  &scratch.A @u32 as i64 0 ==
    [ &!scratch.A 1 as u32 !u32
      Counter lock [
        &!Counter @i64 1 + &!Counter swap !i64
      ]
    ]
    [ ]
  if
;

: isr-lock-atomicity-run ( -- )
  &!systick.LOAD 1 as u32 !u32
  &!systick.VAL 0 as u32 !u32
  &!systick.CTRL 7 as u32 !u32
  Counter lock [
    &!Counter @i64 1 + burn &!Counter swap !i64
  ]
  burn
  Counter lock [
    &Counter @i64 2 ==
      [ ]
      [ 70 testio.write-byte ]
    if
  ]
;

export { isr-lock-atomicity-run };
end;
