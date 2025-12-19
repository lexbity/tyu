module platform/channel;

: platform.channel.make ( -- Chan(i64) )
  # implemented as hosted intrinsic in asm backend
;
: platform.channel.send ( Chan(i64) i64 -- )
  # implemented as hosted intrinsic in asm backend
;
: platform.channel.recv ( Chan(i64) -- i64 )
  # implemented as hosted intrinsic in asm backend
;

end;
