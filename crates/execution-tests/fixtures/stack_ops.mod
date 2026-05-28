module StackOps;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-dup   ( -- ) 7 dup + 14 == check ;
: test-drop  ( -- ) 99 drop 0 0 == check ;
: test-swap  ( -- ) 1 2 swap drop 2 == check ;

: stack-ops-run  ( -- )
  test-dup test-drop test-swap ;

export { stack-ops-run };
end;
