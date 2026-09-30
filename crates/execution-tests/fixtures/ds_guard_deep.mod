module DsGuard;
import platform/testio { testio.write-byte };

: check ( bool -- )
  not [ 70 testio.write-byte ] [ ] if ;

# Data-stack guard baseline (PLAN-WASM-1 L1, guard-on default): a deep but
# in-cap word must run byte-identically under the interpreter and the native
# runner (guards never change well-typed behavior). The `elide` pole (off in
# the REPL by default, §Q4 L1) is a session option that arrives in P3; this
# fixture pins the guard-on trace.
: ds-guard-run ( -- )
  # Peak-32 stack: summon 32 constants, then drop them all.
  1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16
  17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
  drop drop drop drop drop drop drop drop drop drop drop drop drop drop drop drop
  drop drop drop drop drop drop drop drop drop drop drop drop drop drop drop drop
  true check
;

export { ds-guard-run };
end;