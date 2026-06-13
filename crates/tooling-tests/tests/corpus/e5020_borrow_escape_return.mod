# expect: E5020
module Main;
: escape_return ( i64'1 -- Slice(i64) )
  &[
    swap drop
    return
  ]
;
end;
