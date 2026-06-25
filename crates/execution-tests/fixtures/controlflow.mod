module Controlflow;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: choose ( i64 i64 bool -- i64 )
  [ drop ] [ swap drop ] if ;

: countdown ( i64 -- i64 )
  [ dup 0 > ] [ 1 - ] while ;

: branch-value ( i64 -- i64 )
  dup 3 >
    [ 2 * ]
    [ 3 * ]
  if ;

: test-if ( -- )
  10 20 true choose 10 == check
  10 20 false choose 20 == check ;

: test-while ( -- )
  6 countdown 0 == check ;

: test-nested-branch ( -- )
  4 branch-value 8 == check
  3 branch-value 9 == check ;

: controlflow-run ( -- )
  test-if test-while test-nested-branch ;

export { controlflow-run };
end;
