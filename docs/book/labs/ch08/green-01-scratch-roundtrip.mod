# labs/ch08/green-01-scratch-roundtrip.mod — Lab 8.1 (green, QEMU)
# The scratch device: two read-write registers in the emulated aperture.
# Write 42 to A, read it back, then B. The device is NAMED in the
# platform descriptor; the source never mentions an address.
# Expected: under QEMU (x86_64-unknown-none), all checks green, S marker.
module ScratchRT;
import platform/testio { testio.write-byte };

register-map Scratch
  0x00 A u32 rw volatile
  0x04 B u32 rw volatile
end;

const scratch = Scratch @ board.scratch;

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: scratch-rt-run ( -- )
  &!scratch.A 42 as u32 !u32
  &scratch.A @u32 as i64 42 == check
  &!scratch.B 100 as u32 !u32
  &scratch.B @u32 as i64 100 == check ;

export { scratch-rt-run };
end;
