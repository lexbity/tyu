module LocalsSlices;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: bind-one ( i64 -- i64 )
  => x
  x 1 + ;

: bind-two ( i64 i64 -- i64 )
  => rhs
  => lhs
  lhs rhs * ;

: branch-with-local ( i64 -- i64 )
  => x
  x 10 >
    [ x 2 * ]
    [ x 3 * ]
  if ;

: test-local-bindings ( -- )
  41 bind-one 42 == check
  6 7 bind-two 42 == check ;

: test-local-branch ( -- )
  11 branch-with-local 22 == check
  9 branch-with-local 27 == check ;

: locals-slices-run ( -- )
  test-local-bindings test-local-branch ;

export { locals-slices-run };
end;
