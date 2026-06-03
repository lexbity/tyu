module platform/mem;
type Region;
type RegionRef;
type RegionRefMut;

: platform.mem.region-create ( usize -- Region )
  # hosted: runtime-provided (region allocator)
;
: platform.mem.region-alloc ( Region usize -- ptr_mut ) !{alloc}
  # hosted: runtime-provided (region allocator)
;
: platform.mem.region-reset ( Region -- )
  # hosted: runtime-provided (region allocator)
;
: platform.mem.region-destroy ( Region -- )
  # hosted: runtime-provided (region allocator)
;

end;
