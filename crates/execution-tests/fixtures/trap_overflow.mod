module TrapOverflow;
import platform/testio { testio.write-byte };

: recurse ( -- )
  recurse ;

: trap-overflow-run ( -- )
  recurse
  83 testio.write-byte
  10 testio.write-byte ;

export { trap-overflow-run };
end;
