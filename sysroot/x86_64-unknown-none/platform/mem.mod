module platform/mem;
type Region;
type RegionRef;
type RegionRefMut;

: platform.mem.region-create ( usize -- Region )
  # bare-metal: stub — wire a static arena to activate
;
: platform.mem.region-alloc ( Region usize -- ptr_mut )
  # bare-metal: stub
;
: platform.mem.region-reset ( Region -- )
  # bare-metal: stub
;
: platform.mem.region-destroy ( Region -- )
  # bare-metal: stub
;

end;
