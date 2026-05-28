format ELF64

section '.text' executable
extrn __lang_trap
extrn __lang_trap_loc
extrn __stack_overflow
extrn __mmio_mem
extrn __chan_next
extrn __chan_inuse
extrn __chan_head
extrn __chan_tail
extrn __chan_buf
extrn __chan_wait_recv_head
extrn __chan_wait_recv_tail
extrn __chan_wait_recv_buf
extrn __chan_wait_send_head
extrn __chan_wait_send_tail
extrn __chan_wait_send_buf
extrn __task_current
extrn __task_state
extrn __task_g_head
extrn __task_g_tail
extrn __task_g_buf
extrn __region_next
extrn __region_base
extrn __region_size
extrn __region_off
extrn __task_spawn
extrn __task_join
extrn __task_yield
extrn __task_sleep_ms
extrn __task_sleep_us

extrn w_61726974686d657469632d72756e
extrn w_737461636b2d6f70732d72756e
extrn w_74657374696f2e77726974652d62797465
extrn w_74657374696f2e77726974652d737472
extrn w_74657374696f2e65786974

public w_656d69742d646f6e65
w_656d69742d646f6e65:
  jmp .b0_0
.b0_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 83
  add r15, 8
  call w_74657374696f2e77726974652d62797465
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 10
  add r15, 8
  call w_74657374696f2e77726974652d62797465
  jmp .endword_0
.endword_0:
  ret

public w_6d61696e
w_6d61696e:
  sub rsp, 16
  jmp .b1_0
.b1_0:
  call w_61726974686d657469632d72756e
  call w_737461636b2d6f70732d72756e
  call w_656d69742d646f6e65
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 0
  add r15, 8
  sub r15, 8
  mov rax, [r15]
  mov [rsp+8], rax
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov rax, [rsp+8]
  mov [r15], rax
  add r15, 8
  jmp .endword_1
.endword_1:
  add rsp, 16
  ret
