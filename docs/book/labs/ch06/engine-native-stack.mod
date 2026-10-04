# labs/ch06/engine-native-stack.mod — Lab 6.5 (engine, boundary)
# countdown's data-stack bound is finite (net 0, high 1) — the proof holds.
# But the compiler does not yet flatten this recursion to a loop: every
# level is a native call, and at depth 100,000,000 the OS kills the
# process. No trap, no diagnostic: this is the unproven half.
# Expected: compiles cleanly; data-stack proof holds; native stack dies
# (Segmentation fault); tyu run reports NO_COMPLETION.
module NativeBoundary;
import platform/linux { platform.io.log };

: countdown ( i64 -- i64 )
  dup 0 <= [ ] [ 1 - countdown ] if ;

: main ( -- i64 )
  100000000 countdown drop
  "S\n" platform.io.log
  0 ;
export { main };
end;
