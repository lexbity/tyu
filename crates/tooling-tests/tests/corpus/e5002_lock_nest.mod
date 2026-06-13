# expect: E5002
module Main;
resource R;
: nested ( -- )
  R lock [ lock [ ] ]
;
end;
