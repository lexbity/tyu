module IsrLockAtomicity;
import platform/testio { testio.write-byte };

resource Counter : i64 = 0 ceiling 1;

register-map SysTick
  0x00 CTRL u32 rw volatile
  0x04 LOAD u32 rw volatile
  0x08 VAL u32 rw volatile
end;

const systick = SysTick @ 0xE000E010;

: burn ( -- )
  0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop
  0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop
  0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop 0 drop ;

@interrupt(SysTick)
: on_tick ( -- )
  Counter lock [
    &!Counter @i64 1 + &!Counter swap !i64
  ]
;

: main ( -- i64 )
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
  83 testio.write-byte
  10 testio.write-byte
  0
;
end;
