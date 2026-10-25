# PLAN-WASM-2 B1: arithmetic operations test.
# Expected: compiles to valid wasm, returns 55.
module Arithmetic;

: main ( -- i64 )
  10 20 + 5 * 3 + ;

export { main };
end;
