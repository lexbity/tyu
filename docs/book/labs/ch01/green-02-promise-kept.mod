# labs/ch01/green-02-promise-kept.mod — Lab 1.2 (green)
# Expected: two "level accepted" lines, the completion marker, exit 0.
module Level;
import platform/linux { platform.io.log };

: set-level ( i64 -- )
  needs [ dup 0 >= ]
  => level
  "level accepted\n" platform.io.log ;

: main ( -- i64 )
  42 set-level
  100 set-level
  "S\n" platform.io.log
  0 ;

export { main };
end;
