# expect: E5010
module Main;
iso Msg;
: bad_dup ( Msg -- Msg Msg ) dup ;
end;
