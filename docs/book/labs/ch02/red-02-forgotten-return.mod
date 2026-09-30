# labs/ch02/red-02-forgotten-return.mod — Lab 2.6 (red)
# main calls twice, which leaves TWO values; the declaration promises ONE.
# Note: the error does not name the word; see §2.4 for the
# `langc --emit=tc` idiom that shows where the counts go wrong.
module ForgottenReturn;

: dice ( -- i64 )
  6 ;

: main ( -- i64 )
  dice
  dice
  "S\n" platform.io.log
  0 ;

import platform/linux { platform.io.log };
export { main };
end;
