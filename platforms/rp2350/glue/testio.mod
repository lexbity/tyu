module platform/testio;
: testio.write-byte ( i64 -- )
  # RP2350: implemented by the metal runtime asm (UART0 diag), metal.trust
;
: testio.write-str ( str -- )
  # RP2350: implemented by the metal runtime asm (UART0 diag), metal.trust
;
: testio.exit ( i64 -- )
  # RP2350: implemented by the metal runtime asm (diag + halt), metal.trust
;
export { testio.write-byte testio.write-str testio.exit };
end;
