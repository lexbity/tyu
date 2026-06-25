module Trap10Arm;
import platform/testio { testio.write-byte };

: recurse ( -- )
  recurse ;

: trap-10-arm-run ( -- )
  recurse
  83 testio.write-byte
  10 testio.write-byte ;

export { trap-10-arm-run };
end;
