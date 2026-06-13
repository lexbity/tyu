# expect: ok
module Main;
: run_yield ( -- )
  [ platform.task.yield ] platform.task.run
;
end;
