.syntax unified
.thumb

.section .text, "x", %progbits
.extern __lang_load_and_run
.extern __lang_after_main
.extern __lang_trap
.extern __lang_ds_base
.extern __lang_ds_limit

.global __lang_entry
.type __lang_entry, %function
.thumb_func
__lang_entry:
    bl __lang_load_and_run
1:
    b 1b

.global __lang_call_loaded_main
.type __lang_call_loaded_main, %function
.thumb_func
__lang_call_loaded_main:
    push {lr}
    ldr r4, =__lang_ds_base
    ldr r5, =__lang_ds_limit
    @ The loaded module is Thumb code, but the loader registers export addresses
    @ without the Thumb bit. Set it so `blx` stays in Thumb mode (a clear bit 0
    @ would switch to ARM and execute the Thumb bytes as garbage).
    orr r0, r0, #1
    blx r0
    subs r4, r4, #8
    ldr r0, [r4]
    ldr r1, [r4, #4]
    pop {pc}

.global __lang_exit_code
.type __lang_exit_code, %function
.thumb_func
__lang_exit_code:
    ldr r4, =__lang_ds_base
    ldr r5, =__lang_ds_limit
    str r0, [r4]
    str r1, [r4, #4]
    adds r4, r4, #8
    b __lang_after_main

.global __lang_loader_trap
.type __lang_loader_trap, %function
.thumb_func
__lang_loader_trap:
    ldr r4, =__lang_ds_base
    ldr r5, =__lang_ds_limit
    b __lang_trap
