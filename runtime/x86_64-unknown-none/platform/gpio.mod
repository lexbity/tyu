module platform/gpio;
: platform.gpio.init ( usize usize -- )
  # implementation: metal runtime asm (declared in the pack descriptor)
;
: platform.gpio.write ( usize bool -- )
  # implementation: metal runtime asm (declared in the pack descriptor)
;
: platform.gpio.read ( usize -- bool )
  # implementation: metal runtime asm (declared in the pack descriptor)
;
export { platform.gpio.init platform.gpio.write platform.gpio.read };
end;
