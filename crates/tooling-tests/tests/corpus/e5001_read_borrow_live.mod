# expect: E5001
module Main;
: read_borrow_yield ( i64'4 -- i64'4 ) performs {suspend}
  &[
    platform.task.yield drop
  ]
;
end;
