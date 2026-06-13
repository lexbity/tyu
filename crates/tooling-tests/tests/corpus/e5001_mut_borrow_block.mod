# expect: E5001
module Main;
: mut_borrow_yield ( i64'4 -- i64'4 )
  &![
    drop platform.task.yield
  ]
;
end;
