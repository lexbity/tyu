# PLAN-RELEASE-1 smoke — red fixture (ci/smoke/bad.mod).
#
# Negative contract: `underflow` ensures its result is non-negative, but the
# body violates the promise (-1). A healthy toolchain MUST fail this run with
# the contract-violation condition (trap code 20 / CONTRACT_FAIL — see the
# error registry appendix-b), never exit 0. The S6 install-verify job asserts
# the exact error code.
module Bad;
: underflow ( -- i64 )
  ensures [ dup 0 >= ]
  0 1 - ;

: main ( -- i64 ) underflow ;
export { main };
end;