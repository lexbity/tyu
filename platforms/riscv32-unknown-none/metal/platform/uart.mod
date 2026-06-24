module platform/uart;
:: platform.uart.init ( usize -- )
:: platform.uart.tx ( u8 -- )
:: platform.uart.rx ( -- u8 bool )

export { platform.uart.init platform.uart.tx platform.uart.rx }

end;
