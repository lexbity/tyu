# Slice-8 verification corpus — modules the CI gate builds under the
# verifier discipline (static-verification.md §14 P8). Every open obligation
# must be justified in ci/verify-allowlist.txt; entries that stop occurring
# fail the gate (self-cleaning).
module Clean;
# No subtype casts, no contracts: this image's obligation set is empty, so it
# builds under --verify-policy=no-open with nothing to allowlist.
: main ( -- i64 )
  1 2 + 3 + 4 + ;
export { main };
end;