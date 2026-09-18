module MmioStrategiesRiscv;
import platform/testio { testio.write-byte };

register-map Strategy
  0x00 CTRL u32 rw
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

: mmio-strategies-riscv-run ( -- )
  # w1s: write-1-set ORs into SETBITS (RMW or).
  &!strategy.SETBITS 0x5 as u32 !u32
  &strategy.SETBITS @u32 as i64 0x5 == check
  &!strategy.SETBITS 0x2 as u32 !u32
  &strategy.SETBITS @u32 as i64 0x7 == check

  # w1c: seed STATUS via a plain write, then write-1-clear a subset (RMW and).
  &!seed.STATUS 0x7 as u32 !u32
  &!strategy.STATUS 0x4 as u32 !u32
  &strategy.STATUS @u32 as i64 0x3 == check
  &!strategy.STATUS 0x0 as u32 !u32
  &strategy.STATUS @u32 as i64 0x3 == check

  # FIFO effectful read: each @u8 is one access.
  &strategy.FIFO @u8 drop
  &strategy.FIFO @u8 drop ;

export { mmio-strategies-riscv-run };
end;