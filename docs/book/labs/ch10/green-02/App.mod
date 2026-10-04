# labs/ch10/green-02/App.mod — Lab 10.2 (green): the caller, no
# implementation present anywhere in this directory.
module App;
import Math { double };

: caller ( i64 -- i64 ) double double ;

: main ( -- i64 ) 0 ;
export { main };
end;
