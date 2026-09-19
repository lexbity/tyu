module MmioStrategiesX86;
import platform/testio { testio.write-byte };

register-map Strategy
  0x00 CTRL u32 rw { ctrl_low 0..8 u16 rw }
  0x04 STATUS u32 rw
  0x08 SETBITS u32 rw
  0x10 FIFO u8 rw
end;

register-map StrategySeed
  0x04 STATUS u32 rw
end;

const strategy = Strategy @ board.strategy;
const seed = StrategySeed @ board.strategy_seed;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: mmio-strategies-x86-run ( -- )
  # w1s: write-1-set ORs into SETBITS (RMW or).
  &!strategy.SETBITS 0x5 as u32 !u32
  &strategy.SETBITS @u32 as i64 0x5 == check
  &!strategy.SETBITS 0x2 as u32 !u32
  &strategy.SETBITS @u32 as i64 0x7 == check
  # w1s idempotent with 0: writing 0 sets nothing.
  &!strategy.SETBITS 0x0 as u32 !u32
  &strategy.SETBITS @u32 as i64 0x7 == check

  # w1c: seed STATUS via a plain write, then write-1-clear a subset (RMW bic).
  &!seed.STATUS 0x7 as u32 !u32
  &!strategy.STATUS 0x4 as u32 !u32
  &strategy.STATUS @u32 as i64 0x3 == check
  &!strategy.STATUS 0x0 as u32 !u32
  &strategy.STATUS @u32 as i64 0x3 == check

  # field load/store: the field RMW cell (P5 matrix field-ld/field-st).
  strategy.CTRL.ctrl_low 0x5 as u16 !
  strategy.CTRL.ctrl_low @ as i64 0x5 == check

  # FIFO effectful read: each @u8 is one access.
  &strategy.FIFO @u8 drop
  &strategy.FIFO @u8 drop ;

export { mmio-strategies-x86-run };
end;