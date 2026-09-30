# labs/ch08/red-04-bitfield-addressable.mod — Lab 8.6 (red)
# A bitfield is a view into a register, not a place: borrowing its
# address is illegal. Expected: rejected at compile time — E3608.
module BitfieldAddressable;

register-map Strategy
  0x00 CTRL u32 rw { ctrl_low 0..8 u16 rw }
end;

const strategy = Strategy @ board.strategy ;

: f ( -- ) &strategy.CTRL.ctrl_low @u16 drop ;

: main ( -- i64 ) 0 ;
export { main };
end;
