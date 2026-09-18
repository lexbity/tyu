module PtrAddrRiscv;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 FIRST u32 rw
  0x04 SECOND u32 rw
end;

const scratch = Scratch @ board.ptrscratch;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-address-round-trip ( -- )
  &!scratch.FIRST 41 as u32 !u32
  &!scratch.SECOND 1 as u32 !u32
  &scratch.FIRST @u32 as i64 &scratch.SECOND @u32 as i64 + 42 == check ;

: test-mutable-place ( -- )
  &!scratch.FIRST &scratch.FIRST @u32 as i64 1 + as u32 !u32
  &scratch.FIRST @u32 as i64 42 == check ;

: ptr-addr-riscv-run ( -- )
  test-address-round-trip test-mutable-place ;

export { ptr-addr-riscv-run };
end;
