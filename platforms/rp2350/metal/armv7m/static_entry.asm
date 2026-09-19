.syntax unified
.thumb

.section .text, "x", %progbits
.extern w_1f5962a2ce9803c8

.global __lang_entry
.type __lang_entry, %function
.thumb_func
__lang_entry:
    push {lr}
    bl w_1f5962a2ce9803c8
    pop {pc}
