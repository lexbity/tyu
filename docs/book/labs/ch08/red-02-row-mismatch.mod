# labs/ch08/red-02-row-mismatch.mod — Lab 8.4 (red)
# The map declares A as ro; the descriptor says rw. The descriptor wins:
# rows must match, name by name, width by width, access by access.
# Expected: rejected at compile time — E3647.
module RowMismatch;

register-map Scratch
  0x00 A u32 ro volatile
  0x04 B u32 rw volatile
end;

const scratch = Scratch @ board.scratch ;

: main ( -- i64 ) 0 ;
export { main };
end;
