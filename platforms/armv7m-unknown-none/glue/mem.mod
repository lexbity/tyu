module platform/mem;
type Region;
type RegionRef;
type RegionRefMut;

: platform.mem.region-create ( usize -- Region )
  # ARM: wire a static arena to activate
;
: platform.mem.region-alloc ( Region usize -- ptr_mut ) performs {alloc}
   # ARM: allocation within a region arena
;
: platform.mem.region-reset ( Region -- )
  # ARM: reset region allocation state
;
: platform.mem.region-destroy ( Region -- )
  # ARM: release region resources
;

end;
