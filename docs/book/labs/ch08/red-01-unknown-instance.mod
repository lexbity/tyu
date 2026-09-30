# labs/ch08/red-01-unknown-instance.mod — Lab 8.3 (red)
# board.gpio_io does not exist in THIS platform's descriptor: devices are
# named per board, and the checker resolves the name against the
# descriptor. Expected: rejected at compile time — E3644.
module UnknownInstance;

register-map GPIO
  0x00 OUT_SET u32 wo volatile
  0x20 IN      u32 ro volatile
end;

const gpio = GPIO @ board.gpio_io ;

: main ( -- i64 ) 0 ;
export { main };
end;
