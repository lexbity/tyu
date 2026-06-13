# expect: E5004
module Main;
resource R;
: write ( -- )
  &!R drop
;
end;
