# expect: E5001
module Main;
resource R;
: locked_yield ( -- )
  R lock [ platform.task.yield ]
;
end;
