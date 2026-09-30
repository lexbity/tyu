# labs/ch03/green-01-enums.mod — Lab 3.1 (green)
# An enum is a distinct type: its variants are pushed by name, and a word
# declared to take a State accepts them — while an i64-expecting word will
# not compile against them (that rejection is Lab 3.5).
# Expected: silent clean run (exit 0, marker emitted, no F).
module Enums;
import platform/linux { platform.io.log };

enum State : u8
  Idle = 0x00
  Run  = 0x01
end;

: announce ( State -- ) drop ;

: main ( -- i64 )
  State.Run announce
  State.Idle announce
  "S\n" platform.io.log 0 ;
export { main };
end;
