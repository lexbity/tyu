module platform/mem;
type Region;
type RegionRef;
type RegionRefMut;

: platform.mem.region-create ( usize -- Region )
  # RISC-V: implemented by the metal runtime asm bump allocator (P7, metal.trust)
;
: platform.mem.region-alloc ( Region usize -- ptr_mut ) performs {alloc}
  # RISC-V: implemented by the metal runtime asm bump allocator (P7, metal.trust)
;
: platform.mem.region-reset ( Region -- )
  # RISC-V: implemented by the metal runtime asm bump allocator (P7, metal.trust)
;
: platform.mem.region-destroy ( Region -- )
  # RISC-V: implemented by the metal runtime asm bump allocator (P7, metal.trust)
;

end;
