# expect: E5011
module Main;
iso Msg;
: bad_drop ( Msg -- ) drop ;
end;
