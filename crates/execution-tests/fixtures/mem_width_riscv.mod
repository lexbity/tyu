module MemWidthRiscv;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 B u8 rw
  0x02 H u16 rw
  0x04 W u32 rw
end;

const scratch = Scratch @ 0x80100000;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-u8 ( -- )
  &!scratch.B 127 as u8 !u8
  &scratch.B @u8 as i64 127 == check ;

: test-u16 ( -- )
  &!scratch.H 4660 as u16 !u16
  &scratch.H @u16 as i64 4660 == check ;

: test-u32 ( -- )
  &!scratch.W 42 as u32 !u32
  &scratch.W @u32 as i64 42 == check ;

: mem-width-riscv-run ( -- )
  test-u8 test-u16 test-u32 ;

export { mem-width-riscv-run };
end;
