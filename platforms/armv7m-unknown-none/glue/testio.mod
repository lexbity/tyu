module platform/testio;
: testio.write-byte ( i64 -- )
  # ARM semihosting: pops 8-byte i64 (two DS slots), emits low byte via SYS_WRITEC
;
: testio.write-str ( str -- )
  # ARM semihosting: pops 4-byte str pointer (one DS slot), iterates bytes
;
: testio.exit ( i64 -- )
  # ARM semihosting: pops 8-byte i64 (two DS slots), terminates via SYS_EXIT
;
export { testio.write-byte testio.write-str testio.exit };
end;
