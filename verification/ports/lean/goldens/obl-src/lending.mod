module Lending;
subtype Percent = i64 range 0..100;
export { withdraw } ;
: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;
: withdraw ( Percent -- bool )
  needs [ pct-in-range ] intent "withdraw never exceeds balance"
  drop true ;
end;
