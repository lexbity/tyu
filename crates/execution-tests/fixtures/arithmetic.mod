module Arithmetic;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-add   ( -- ) 2 3 +  5 ==  check ;
: test-sub   ( -- ) 10 3 - 7 ==  check ;
: test-mul   ( -- ) 4 5 *  20 == check ;
: test-lt    ( -- ) 3 5 <        check ;
: test-gt    ( -- ) 7 2 >        check ;
: test-eq    ( -- ) 42 42 ==     check ;

: arithmetic-run  ( -- )
  test-add test-sub test-mul test-lt test-gt test-eq ;

export { arithmetic-run };
end;
