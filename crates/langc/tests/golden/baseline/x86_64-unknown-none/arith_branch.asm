format ELF64

section '.text' executable
extrn __lang_trap
extrn __lang_trap_loc
extrn __stack_overflow
extrn __lang_ds_high
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
extrn __lang_expected_abi_hash


public w_1f5962a2ce9803c8
w_1f5962a2ce9803c8:
  sub rsp, 16
  jmp .b0_0
.b0_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 10
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_2
  mov [__lang_ds_high], r15
.ds_high_2:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 7
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_3
  mov [__lang_ds_high], r15
.ds_high_3:
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  add rax, rcx
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_4
  mov [__lang_ds_high], r15
.ds_high_4:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 3
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_5
  mov [__lang_ds_high], r15
.ds_high_5:
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  imul rax, rcx
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_6
  mov [__lang_ds_high], r15
.ds_high_6:
  sub r15, 8
  mov rax, [r15]
  mov [rsp+8], rax
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov rax, [rsp+8]
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_7
  mov [__lang_ds_high], r15
.ds_high_7:
  jmp .endword_0
.endword_0:
  add rsp, 16
  ret
section '.lang.modinfo'
  db 68,79,77,76,3,0,0,0,171,176,218,26,139,91,240,65,32,0,0,0,4,0,0,0,1,0,0,0,0,0,0,0,77,97,105,110,109,97,105,110,0,0,0,0,200,3,152,206,162,98,89,31,36,0,0,0,60,0,0,0,200,3,152,206,162,98,89,31,0,0,0,0,0,0,0,0
