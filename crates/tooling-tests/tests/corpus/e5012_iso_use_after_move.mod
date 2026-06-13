# expect: E5012
module Main;
iso Msg;
: move_twice ( Msg -- Msg )
  => x
  x x
;
end;
