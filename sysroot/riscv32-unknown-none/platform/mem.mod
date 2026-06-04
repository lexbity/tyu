module platform/mem;
type Region;
type RegionRef;
type RegionRefMut;

: platform.mem.region-create ( usize -- Region )
  # RISC-V: wire a static arena to activate
;
: platform.mem.region-alloc ( Region usize -- ptr_mut ) !{alloc}
   # RISC-V: allocation within a region arena
;
: platform.mem.region-reset ( Region -- )
  # RISC-V: reset region allocation state
;
: platform.mem.region-destroy ( Region -- )
  # RISC-V: release region resources
;

end;
