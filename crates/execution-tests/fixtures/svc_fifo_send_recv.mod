module FifoSendRecv;
import platform/channel;

: expect_42 ( i64 -- i64 ) 42 == not [ 1 ] [ 0 ] if ;

: main ( -- i64 )
  platform.channel.make
  dup
  42 platform.channel.send
  platform.channel.recv
  expect_42
  ;
export { main };
end;
