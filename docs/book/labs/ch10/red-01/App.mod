# labs/ch10/red-01/App.mod — Lab 10.3 (red): the trusting caller
module App;
import Math { double };
: main ( -- i64 ) 21 double drop 0 ;
export { main };
end;
