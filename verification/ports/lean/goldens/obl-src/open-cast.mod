# Slice-8 verification corpus — the book's own both-policies example
# (ch03 "Compile-time discharge"): `150 as Percent` is a *constant*
# out-of-range value. The interval engine proves it (provably_failing), the
# obligation STAYS OPEN (the runtime trap is the cast's semantics — a
# discharge would remove a trap that must fire), so under
# --verify-policy=no-open this module is rejected, and under open-ok it
# builds with the trap retained. Justified in ci/verify-allowlist.txt.
module OpenCast;
subtype Percent = i64 range 0..100;
: main ( -- i64 )
  150 as Percent drop 0 ;
export { main };
end;