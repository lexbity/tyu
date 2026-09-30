# PLAN-RELEASE-1 smoke — green fixture (ci/smoke/hello.mod).
#
# Release-engineering-owned, minimal by design (S5/S6), and target-agnostic:
# no `platform/*` import, so the SAME bytes build and run on the hosted
# target (x86_64-unknown-linux-gnu) AND on bare-metal x86_64-unknown-none
# under QEMU (FR-23) — the canonical smoke for FR-9 (install-verify) and
# FR-20 (container). The smoke contract is the IMAGE exit code: `main`
# returns 0 on success, 1 on a wrong result.
#
# One word carries BOTH sides of a contract pair — the requires clause
# (`needs`, checked after the call, before the body) and the ensures clause
# (`ensures`, checked after the body, before return). Both pass on the smoke
# call (20 22 add-sat = 42), so a healthy toolchain proves the whole
# contract path end to end: a toolchain whose contract checking is broken
# traps (exit code 20 / CONTRACT_FAIL) instead of returning 0.
module Hello;
: add-sat ( i64 i64 -- i64 )
  needs  [ dup dup + 0 >= ]   # requires: inputs sum is non-negative
  ensures [ dup 0 >= ]        # ensures:  result is non-negative
  + ;

: main ( -- i64 )
  20 22 add-sat 42 == [ 0 ] [ 1 ] if ;
export { main };
end;