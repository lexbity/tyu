format ELF64

section '.text' executable
use64

extrn __lang_load_and_run
extrn __lang_after_main
extrn __lang_trap
extrn __lang_ds_base
extrn __lang_ds_limit

public __lang_entry
__lang_entry:
    call __lang_load_and_run
    cli
    hlt

public __lang_call_loaded_main
__lang_call_loaded_main:
    lea r15, [__lang_ds_base]
    lea r14, [__lang_ds_limit]
    call rdi
    sub r15, 8
    mov rax, [r15]
    ret

public __lang_exit_code
__lang_exit_code:
    lea r15, [__lang_ds_base]
    lea r14, [__lang_ds_limit]
    mov [r15], rdi
    add r15, 8
    jmp __lang_after_main

public __lang_loader_trap
__lang_loader_trap:
    lea r15, [__lang_ds_base]
    lea r14, [__lang_ds_limit]
    jmp __lang_trap
