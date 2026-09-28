module Uart7;
register-map UART
  0x018 UARTFR u32 ro volatile
end;
subtype RxBit = i64 range 0..1;
const uart = UART @ board.uart0;
: read-tx-idle ( -- RxBit )
  &uart.UARTFR @u32 drop 1 as RxBit
;
: main ( -- i64 )
  0 ;
export { read-tx-idle };
end;