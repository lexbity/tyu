module MemWidthArm;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 B u8 rw
  0x04 W u32 rw
  0x08 D i64 rw
end;

const scratch = Scratch @ 0x20007000;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-u8 ( -- )
  &!scratch.B 127 as u8 !u8
  &scratch.B @u8 as i64 127 == check ;

: test-u32 ( -- )
  &!scratch.W 305419896 as u32 !u32
  &scratch.W @u32 as i64 305419896 == check ;

: test-i64 ( -- )
  &!scratch.D 7 !i64
  &scratch.D @i64 7 == check ;

: mem-width-arm-run ( -- )
  test-u8 test-u32 test-i64 ;

export { mem-width-arm-run };
end;
