module platform/testio;
: testio.write-byte ( i64 -- )
  # bare-metal: runtime-provided (port 0xe9 debugcon)
;
: testio.write-str ( str -- )
  # bare-metal: runtime-provided (port 0xe9 debugcon, iterates bytes)
;
: testio.exit ( i64 -- )
  # bare-metal: runtime-provided (isa-debug-exit port 0x501)
;
export { testio.write-byte testio.write-str testio.exit };
end;
