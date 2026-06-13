# expect: ok
module Main;
iso Msg;
: handoff ( Msg -- Msg )
  => x
  x
;
end;
