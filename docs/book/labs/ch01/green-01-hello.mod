# labs/ch01/green-01-hello.mod — Lab 1.1 (green)
# Expected: prints one line, emits the completion marker, exits 0.
module Hello;
import platform/linux { platform.io.log };

: main ( -- i64 )
  "assembling in tyu\n" platform.io.log
  "S\n" platform.io.log
  0 ;

export { main };
end;
