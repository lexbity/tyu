# expect: E5003
module Main;
resource R;
: bad ( -- )
  R lock [ 1 ]
;
end;
