module platform/channel;

: platform.channel.make ( -- |i64| )
  # implemented as hosted intrinsic in asm backend
;
: platform.channel.send ( |i64| i64 -- )
  # implemented as hosted intrinsic in asm backend
;
: platform.channel.recv ( |i64| -- i64 )
  # implemented as hosted intrinsic in asm backend
;

end;
