# labs/ch05/red-02-resource-outside-lock.mod — Lab 5.6 (red)
# The grant comes from the lock, and no lock encloses this access.
# Expected: rejected at compile time — E5004 (capability missing).
module OutsideLock;

resource Counter : i64;

: sneak ( -- i64 )
  &!Counter @i64 ;

: main ( -- i64 ) 0 ;
export { main };
end;
