module platform/uart;
: platform.uart.init ( usize -- )
  # RP2350: implemented by the metal runtime asm, metal.trust
;
: platform.uart.tx ( u8 -- )
  # RP2350: implemented by the metal runtime asm, metal.trust
;
: platform.uart.rx ( -- u8 bool )
  # RP2350: implemented by the metal runtime asm, metal.trust
;
export { platform.uart.init platform.uart.tx platform.uart.rx }
end;
