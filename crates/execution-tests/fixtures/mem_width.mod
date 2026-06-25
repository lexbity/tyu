module MemWidth;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 B u8 rw
  0x02 H u16 rw
  0x04 W u32 rw
  0x08 D i64 rw
end;

const scratch = Scratch @ 0x00;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-u8 ( -- )
  &!scratch.B 255 as u8 !u8
  &scratch.B @u8 as i64 255 == check ;

: test-u16 ( -- )
  &!scratch.H 4660 as u16 !u16
  &scratch.H @u16 as i64 4660 == check ;

: test-u32 ( -- )
  &!scratch.W 305419896 as u32 !u32
  &scratch.W @u32 as i64 305419896 == check ;

: test-i64 ( -- )
  &!scratch.D 7 !i64
  &scratch.D @i64 7 == check ;

: mem-width-run ( -- )
  test-u8 test-u16 test-u32 test-i64 ;

export { mem-width-run };
end;
