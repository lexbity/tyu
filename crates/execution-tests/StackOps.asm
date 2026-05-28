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

extrn w_74657374696f2e77726974652d62797465
extrn w_74657374696f2e77726974652d737472
extrn w_74657374696f2e65786974

public w_636865636b
w_636865636b:
  sub rsp, 16
  jmp .b0_0
.b0_0:
  sub r15, 8
  mov rax, [r15]
  mov [rsp+0], rax
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov rax, [rsp+0]
  mov [r15], rax
  add r15, 8
  sub r15, 8
  mov rax, [r15]
  cmp rax, 0
  sete al
  movzx rax, al
  mov [r15], rax
  add r15, 8
  sub r15, 8
  mov rax, [r15]
  cmp rax, 0
  je .b0_2
  jmp .b0_1
.b0_1:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 70
  add r15, 8
  call w_74657374696f2e77726974652d62797465
  jmp .b0_3
.b0_2:
  jmp .b0_3
.b0_3:
  jmp .endword_0
.endword_0:
  add rsp, 16
  ret

public w_746573742d647570
w_746573742d647570:
  jmp .b1_0
.b1_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 7
  add r15, 8
  mov rax, [r15-8]
  lea rcx, [r15+8]
  cmp rcx, r14
  ja __stack_overflow
  mov [r15], rax
  add r15, 8
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  add rax, rcx
  mov [r15], rax
  add r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 14
  add r15, 8
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  cmp rax, rcx
  sete al
  movzx rax, al
  mov [r15], rax
  add r15, 8
  call w_636865636b
  jmp .endword_1
.endword_1:
  ret

public w_746573742d64726f70
w_746573742d64726f70:
  jmp .b2_0
.b2_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 99
  add r15, 8
  sub r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 0
  add r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 0
  add r15, 8
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  cmp rax, rcx
  sete al
  movzx rax, al
  mov [r15], rax
  add r15, 8
  call w_636865636b
  jmp .endword_2
.endword_2:
  ret

public w_746573742d73776170
w_746573742d73776170:
  jmp .b3_0
.b3_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 1
  add r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 2
  add r15, 8
  mov rax, [r15-8]
  mov rcx, [r15-16]
  mov [r15-8], rcx
  mov [r15-16], rax
  sub r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 2
  add r15, 8
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  cmp rax, rcx
  sete al
  movzx rax, al
  mov [r15], rax
  add r15, 8
  call w_636865636b
  jmp .endword_3
.endword_3:
  ret

public w_737461636b2d6f70732d72756e
w_737461636b2d6f70732d72756e:
  jmp .b4_0
.b4_0:
  call w_746573742d647570
  call w_746573742d64726f70
  call w_746573742d73776170
  jmp .endword_4
.endword_4:
  ret
