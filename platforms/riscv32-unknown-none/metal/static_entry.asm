.section .text
.extern w_1f5962a2ce9803c8

.globl __lang_entry
.type __lang_entry, @function
__lang_entry:
    addi sp, sp, -16
    sw ra, 12(sp)
    jal w_1f5962a2ce9803c8
    lw ra, 12(sp)
    addi sp, sp, 16
    ret
