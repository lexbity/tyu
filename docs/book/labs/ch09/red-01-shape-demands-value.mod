# labs/ch09/red-01-shape-demands-value.mod — Lab 9.3 (red)
# The natural-looking factoring: a word that answers "in range?" — but it
# consumes its subject, and a contract predicate must keep the value and
# return value+verdict. Expected: rejected at compile time — E3220.
module NaturalButWrong;

: in-range ( i64 -- bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: judge ( i64 -- i64 )
  needs [ in-range ]
  as i64 ;

: main ( -- i64 ) 0 ;
export { main };
end;
