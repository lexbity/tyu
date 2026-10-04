# labs/ch10/green-01/App.mod — Lab 10.1 (green): the caller
module App;
import Math { double };
import platform/linux { platform.io.log };

: check ( bool -- ) not [ "F\n" platform.io.log ] [ ] if ;

: main ( -- i64 )
  21 double 42 == check
  "S\n" platform.io.log 0 ;
export { main };
end;
