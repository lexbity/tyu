# labs/ch13/green-01-ping.mod — Lab 13.1 (green)
# The field module: a minimal word-set whose .lmod will be packed, signed,
# and (in Lab 13.2) loaded into running firmware through the loader.
# Expected: packs and signs cleanly with the chapter's tool transcript.
module Ping;
import platform/testio { testio.write-byte };
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;
export { main };
end;
