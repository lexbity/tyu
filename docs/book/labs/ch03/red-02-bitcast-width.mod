# labs/ch03/red-02-bitcast-width.mod — Lab 3.6 (red)
# bitcast reinterprets bits and requires equal size: i64 is 8 bytes,
# u32 is 4. Expected: rejected at compile time — E3304.
module BitcastWidth;

: shrink ( i64 -- i64 )
  bitcast u32 as i64 ;

: main ( -- i64 ) 0 ;
export { main };
end;
