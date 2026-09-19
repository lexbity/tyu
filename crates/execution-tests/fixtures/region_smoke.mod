module RegionSmoke;
import platform/mem;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: region-smoke-run ( -- )
  # create a 64-byte region, allocate 16, typed store/load round-trip,
  # then destroy. Exercises the real per-target reference allocator
  # (x86: codegen-inline; arm/riscv: metal.trust bump arena). Any wrong
  # value emits the 'F' failure byte via testio.
  64 as usize platform.mem.region-create
  dup 16 as usize platform.mem.region-alloc
  dup 42 as i64 !i64
  dup @i64 42 == check
  drop
  platform.mem.region-destroy
;

export { region-smoke-run };
end;