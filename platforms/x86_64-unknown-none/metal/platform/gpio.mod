module platform/gpio;
: platform.gpio.init ( usize usize -- )
  # bare-metal: QEMU stub backed by runtime assembly
;
: platform.gpio.write ( usize bool -- )
  # bare-metal: QEMU stub backed by runtime assembly
;
: platform.gpio.read ( usize -- bool )
  # bare-metal: QEMU stub backed by runtime assembly
;
export { platform.gpio.init platform.gpio.write platform.gpio.read };
end;
