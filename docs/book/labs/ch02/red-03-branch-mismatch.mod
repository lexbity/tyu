# labs/ch02/red-03-branch-mismatch.mod — Lab 2.7 (red)
# Expected: rejected at compile time — E3246 (branch depth mismatch).
module BranchMismatch;

: main ( -- i64 )
  true [ 0 ] [ 0 0 ] if drop
  0 ;

export { main };
end;
