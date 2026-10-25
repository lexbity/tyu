# PLAN-WASM-2 B1: stack manipulation operations test.
# Expected: compiles to valid wasm, returns 3.
module StackOps;

: main ( -- i64 )
  1 2 3 drop drop ;

export { main };
end;
