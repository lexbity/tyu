# expect: E5001
module Main;
resource R;
: run_under_lock ( -- )
  R lock [ [ platform.task.yield ] platform.task.run ]
;
end;
