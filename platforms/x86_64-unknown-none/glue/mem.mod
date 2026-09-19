module platform/mem;
type Region;
type RegionRef;
type RegionRefMut;

: platform.mem.region-create ( usize -- Region )
  # implemented by the codegen-inline allocator (P7, region.rs)
;
: platform.mem.region-alloc ( Region usize -- ptr_mut ) performs {alloc}
   # implemented by the codegen-inline allocator (P7, region.rs)
;
: platform.mem.region-reset ( Region -- )
  # implemented by the codegen-inline allocator (P7, region.rs)
;
: platform.mem.region-destroy ( Region -- )
  # implemented by the codegen-inline allocator (P7, region.rs)
;

end;
