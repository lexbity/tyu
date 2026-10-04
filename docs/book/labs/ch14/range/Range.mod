# labs/ch14/range/Range.mod — Lab 14.4 (red)
# The chapter 9 factored contract: a needs clause that computes through a
# helper word. The obligation pass refuses it — predicates must be
# arithmetic over values in view. Expected: E3312 at pass-1.
module Range;
: bounded-by ( i64 i64 -- i64 bool )
  => hi dup hi <= ;
: set-range ( i64 -- i64 )
  needs [ dup 0 >= [ 100 bounded-by ] [ false ] if ]
  as i64 ;
: main ( -- i64 ) 50 set-range drop 0 ;
export { main };
end;
