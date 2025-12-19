format ELF64

section '.text' executable
public _start
public __lang_start
public __lang_trap
public __lang_trap_loc
public __stack_overflow

extrn w_6d61696e ; main

_start:
__lang_start:
  mov r15, __lang_ds_base
  mov r14, __lang_ds_limit
  call w_6d61696e
  sub r15, 8
  mov rdi, [r15]
  and rdi, 0xff
  mov rax, 60
  syscall

__lang_trap:
__lang_trap_loc:
  mov rax, 60
  syscall

__stack_overflow:
  mov rdi, 10
  jmp __lang_trap

section '.bss' writeable
public __chan_next
public __chan_head
public __chan_tail
public __chan_buf
__chan_next dq 0
__chan_head rq 16
__chan_tail rq 16
__chan_buf rq 1024
__mmio_mem rb 65536
__lang_ds_base rb 65536
__lang_ds_limit:
