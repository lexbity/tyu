# Slice-8 verification corpus — the canonical embedded event loop: diverging
# (a `loop`) but finite-high (§3.2). Obligation set is empty (no subtype
# casts, no contracts), so no-open holds and the module also exercises the
# image story's DIVERGE-is-not-the-criterion rule (pinned in slice 7).
module EventLoop;
: poll ( -- )
  1 drop ;
: event-loop ( -- )
  [ poll ] loop ;
: main ( -- i64 )
  event-loop 0 ;
export { main };
end;