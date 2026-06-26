format ELF64

section '.text' executable
use64

extrn w_1f5962a2ce9803c8    ; main ( -- i64 )

public __lang_entry
__lang_entry:
    call w_1f5962a2ce9803c8
    ret
