.section .text
.extern __lang_load_and_run
.extern __lang_after_main
.extern __lang_trap
.extern __lang_ds_base
.extern __lang_ds_limit

.globl __lang_entry
.type __lang_entry, @function
__lang_entry:
    jal __lang_load_and_run
1:
    j 1b

.globl __lang_call_loaded_main
.type __lang_call_loaded_main, @function
__lang_call_loaded_main:
    addi sp, sp, -16
    sw ra, 12(sp)
    la s2, __lang_ds_base
    la s3, __lang_ds_limit
    jalr a0
    addi s2, s2, -8
    lw a0, 0(s2)
    lw a1, 4(s2)
    lw ra, 12(sp)
    addi sp, sp, 16
    ret

.globl __lang_exit_code
.type __lang_exit_code, @function
__lang_exit_code:
    la s2, __lang_ds_base
    la s3, __lang_ds_limit
    sw a0, 0(s2)
    sw a1, 4(s2)
    addi s2, s2, 8
    j __lang_after_main

.globl __lang_loader_trap
.type __lang_loader_trap, @function
__lang_loader_trap:
    la s2, __lang_ds_base
    la s3, __lang_ds_limit
    j __lang_trap
