module Uart7;
register-map UART
  0x018 UARTFR u32 ro volatile
end;
subtype RxByte = i64 range 0..255;
const uart = UART @ board.uart0;
: read-tx-idle ( -- RxByte )
  &uart.UARTFR @u32 as i64 as RxByte
;
: main ( -- i64 )
  read-tx-idle ;
export { read-tx-idle };
end;