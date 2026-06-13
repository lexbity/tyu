# expect: ok
module Main;
: example ( i64'4 -- i64'4 ) performs {suspend}
  &[
    drop
  ]
  platform.task.yield
;
end;
