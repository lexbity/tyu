# labs/ch08/red-05-needs-descriptor.mod — Lab 8.7 (red)
# MMIO without a platform descriptor cannot be checked, so it cannot be
# compiled: build this module without --platform and the compiler refuses.
# Expected: rejected at compile time — E3640.
module NeedsDescriptor;

register-map Scratch
  0x00 A u32 rw volatile
end;

const scratch = Scratch @ board.scratch ;

: main ( -- i64 ) 0 ;
export { main };
end;
