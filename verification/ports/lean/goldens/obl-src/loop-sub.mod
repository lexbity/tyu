module LoopSub;
subtype Percent = i64 range 0..100;
: bounded-count ( i64 -- Percent )
  dup 100 > [ drop 100 ] [ ] if
  dup 0 < [ drop 0 ] [ ] if
  [ dup 5 < ] [ 1 + ] while
  as Percent ;
: main ( -- i64 )
  3 bounded-count drop 0 ;
export { main };
end;
