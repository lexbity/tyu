# labs/ch07/green-02-channel-custody.mod — Lab 7.2 (green)
# send CONSUMES the value (a move, not a copy); recv receives it on the
# other end of the same channel. Custody transfers, end to end.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Custody;
import platform/linux { platform.io.log };
import platform/channel;

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  platform.channel.make dup
  42 platform.channel.send
  platform.channel.recv 42 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
