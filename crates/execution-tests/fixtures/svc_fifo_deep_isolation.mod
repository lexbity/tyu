module FifoDeepIsolation;
import platform/channel;

: expect_7 ( i64 -- i64 ) 7 == not [ 1 ] [ 0 ] if ;
: expect_9 ( i64 -- i64 ) 9 == not [ 1 ] [ 0 ] if ;

: main ( -- i64 )
  platform.channel.make
  dup
  7 platform.channel.send
  platform.channel.recv
  expect_7
  platform.channel.make
  dup
  dup
  9 platform.channel.send
  11 platform.channel.send
  platform.channel.recv
  expect_9
  +
  ;
export { main };
end;
