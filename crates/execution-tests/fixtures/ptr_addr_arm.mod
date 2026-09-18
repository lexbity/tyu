module PtrAddrArm;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 FIRST i64 rw
  0x08 SECOND i64 rw
end;

const scratch = Scratch @ board.ptrscratch;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-address-round-trip ( -- )
  &!scratch.FIRST 41 !i64
  &!scratch.SECOND 1 !i64
  &scratch.FIRST @i64 &scratch.SECOND @i64 + 42 == check ;

: test-mutable-place ( -- )
  &!scratch.FIRST &scratch.FIRST @i64 1 + !i64
  &scratch.FIRST @i64 42 == check ;

: ptr-addr-arm-run ( -- )
  test-address-round-trip test-mutable-place ;

export { ptr-addr-arm-run };
end;
