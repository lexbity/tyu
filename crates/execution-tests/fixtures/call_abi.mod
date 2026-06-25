module CallAbi;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: sum7 ( i64 i64 i64 i64 i64 i64 i64 -- i64 )
  + + + + + + ;

: add3 ( i64 i64 i64 -- i64 )
  + + ;

: call-chain ( i64 -- i64 )
  2 3 4 5 6 7 sum7 ;

: test-many-args ( -- )
  1 2 3 4 5 6 7 sum7 28 == check ;

: test-nested-call ( -- )
  1 call-chain 28 == check ;

: test-return-value ( -- )
  10 20 12 add3 42 == check ;

: call-abi-run ( -- )
  test-many-args test-nested-call test-return-value ;

export { call-abi-run };
end;
