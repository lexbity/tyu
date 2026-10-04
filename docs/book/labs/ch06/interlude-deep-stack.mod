# labs/ch06/interlude-deep-stack.mod — Lab 6.6 (interlude, QEMU)
# The repo fixture, run under QEMU (x86_64-unknown-none): the runtime
# measures its own peak and emits the H marker; the harness checks it
# against the compiler-claimed bound.
module DeepStack;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

: test-peak-10 ( -- )
  1 2 3 4 5 6 7 8 9 10
  drop drop drop drop drop
  drop drop drop drop drop ;

: test-symmetric-branch ( -- )
  true
  [ 1 2 3 4 5 drop drop drop drop drop ]
  [ 6 7 8 9 10 drop drop drop drop drop ]
  if ;

: test-loop-temp ( -- )
  1 [ dup 0 > ] [ 1 - ] while drop ;

: deep-stack-run ( -- )
  test-peak-10 test-symmetric-branch test-loop-temp ;

export { deep-stack-run };
end;
