module Trap10Riscv;
import platform/testio { testio.write-byte };

: recurse ( -- )
  recurse ;

: trap-10-riscv-run ( -- )
  recurse
  83 testio.write-byte
  10 testio.write-byte ;

export { trap-10-riscv-run };
end;
