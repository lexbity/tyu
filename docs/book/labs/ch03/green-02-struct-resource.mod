# labs/ch03/green-02-struct-resource.mod — Lab 3.2 (green)
# A struct is a distinct composite type. There is no literal construction
# on the current tree, so a Point arrives as a word parameter; words that
# take and return a Point compile and run like any other word.
# Expected: silent clean run (exit 0, marker emitted, no F).
module Structs;
import platform/linux { platform.io.log };

struct Point
  x : i32
  y : i32
end;

: pass ( Point -- Point ) ;

: main ( -- i64 )
  "S\n" platform.io.log 0 ;
export { main };
end;
