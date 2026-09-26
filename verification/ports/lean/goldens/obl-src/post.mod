module Post;
subtype Percent = i64 range 0..100;
: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;
: m ( Percent -- Percent )
  ensures [ pct-in-range ]
  1 + as Percent ;
export { m };
end;
