# labs/ch10/red-02/Math.mod — Lab 10.4 (red): hidden is not exported
module Math;

: hidden ( i64 -- i64 ) 1 + ;
: double ( i64 -- i64 ) 2 * ;

export { double };
end;
