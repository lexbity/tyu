module MmioSmokeRiscv;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;

const scratch = Scratch @ board.scratch;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: mmio-smoke-riscv-run ( -- )
  &!scratch.A 42 as u32 !u32
  &scratch.A @u32 as i64 42 == check
  &!scratch.B 100 as u32 !u32
  &scratch.B @u32 as i64 100 == check ;

export { mmio-smoke-riscv-run };
end;
