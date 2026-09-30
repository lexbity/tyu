# labs/ch14/anon/Anon.mod — Lab 14.5 (red)
# An anonymous inline needs predicate: the obligation exists, but its
# provenance is opaque, and the renderer declines to fabricate a
# statement. Expected: 0 rendered, 1 omitted; the obligation open with
# its runtime check retained.
module Anon;
: f ( i64 -- ) needs [ dup 0 >= ] drop ;
: main ( -- i64 ) 5 f 0 ;
export { main };
end;
