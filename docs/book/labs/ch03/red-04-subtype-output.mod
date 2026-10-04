# labs/ch03/red-04-subtype-output.mod — Lab 3.8 (red)
# Subtype-typed values do not cross word boundaries: a word may take a
# Percent, but it may not promise one. Expected: rejected at compile
# time — E3220 (declared output vs body, which produces the base i64).
module SubtypeOutput;

subtype Percent = i64 range 0 .. 100;

: make-percent ( -- Percent )
  50 as Percent ;

: main ( -- i64 ) make-percent as i64 0 ;
export { main };
end;
