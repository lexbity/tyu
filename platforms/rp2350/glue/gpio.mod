module platform/gpio;
: platform.gpio.init ( usize usize -- )
  # RP2350: implemented by the metal runtime asm, metal.trust
;
: platform.gpio.write ( usize bool -- )
  # RP2350: implemented by the metal runtime asm, metal.trust
;
: platform.gpio.read ( usize -- bool )
  # RP2350: implemented by the metal runtime asm, metal.trust
;
export { platform.gpio.init platform.gpio.write platform.gpio.read }
end;
