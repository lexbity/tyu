# PLAN-WASM-2 B1: basic constant test — the backend corpus' ground zero.
# Expected: compiles to valid wasm, returns 42.
module BasicConst;

: main ( -- i64 )
  42 ;

export { main };
end;
