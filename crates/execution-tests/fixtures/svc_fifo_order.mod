module FifoOrder;
import platform/channel;

: expect_7 ( i64 -- i64 ) 7 == not [ 1 ] [ 0 ] if ;

: main ( -- i64 )
  platform.channel.make
  dup
  dup
  7 platform.channel.send
  9 platform.channel.send
  platform.channel.recv
  expect_7
  ;
export { main };
end;
