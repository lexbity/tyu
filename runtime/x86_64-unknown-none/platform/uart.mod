module platform/uart;
: platform.uart.init ( usize -- )
  # bare-metal: QEMU debugcon needs no init
;
: platform.uart.tx ( u8 -- )
  # bare-metal: runtime assembly writes to QEMU debugcon
;
: platform.uart.rx ( -- u8 bool )
  # bare-metal: runtime assembly reports no input available
;
export { platform.uart.init platform.uart.tx platform.uart.rx };
end;
