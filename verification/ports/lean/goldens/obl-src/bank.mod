module Bank;
subtype Percent = i64 range 0..100;
subtype Counter = i64 range 0..1000000;
: clamp ( i64 -- Percent )
  dup 100 > [ drop 100 ] [ ] if
  dup 0 < [ drop 0 ] [ ] if
  as Percent ;
: bounded_inc ( Percent -- Percent ) 1 + as Percent ;
: main ( -- Counter ) 50 as Percent bounded_inc as Counter ;
end;
