# labs/ch12/red-03-channel-absent.mod — Lab 12.6 (red)
# platform.channel exists only in the hosted platform's interface. Build
# this for the board (x86_64-unknown-none) and the import fails: the
# platform's interface is the contract. Expected: E2201.
module ChannelAbsent;
import platform/channel;

: f ( -- |i64| ) platform.channel.make ;

: main ( -- i64 ) 0 ;
export { main };
end;
