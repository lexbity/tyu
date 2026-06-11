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


public w_9c4c7ccd2b84562d
w_9c4c7ccd2b84562d:
  sub rsp, 32
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
  cmp r15, [__lang_ds_high]
  jna .ds_high_0
  mov [__lang_ds_high], r15
.ds_high_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 42
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_1
  mov [__lang_ds_high], r15
.ds_high_1:
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  add rax, rcx
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_2
  mov [__lang_ds_high], r15
.ds_high_2:
  sub r15, 8
  mov rax, [r15]
  mov [rsp+16], rax
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov rax, [rsp+16]
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_3
  mov [__lang_ds_high], r15
.ds_high_3:
  jmp .endword_0
.endword_0:
  add rsp, 32
  ret

public w_1f5962a2ce9803c8
w_1f5962a2ce9803c8:
  sub rsp, 16
  jmp .b1_0
.b1_0:
  mov rax, __lang_str_2
  lea rcx, [r15+8]
  cmp rcx, r14
  ja __stack_overflow
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_4
  mov [__lang_ds_high], r15
.ds_high_4:
  sub r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 7
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_5
  mov [__lang_ds_high], r15
.ds_high_5:
  call w_9c4c7ccd2b84562d
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
  jna .ds_high_6
  mov [__lang_ds_high], r15
.ds_high_6:
  jmp .endword_1
.endword_1:
  add rsp, 16
  ret

segment readable

__lang_str_2:
  dq __lang_str_2_bytes
  dq 2
__lang_str_2_bytes db 111,107
section '.lang.modinfo'
  db 68,79,77,76,2,0,0,0,43,32,167,58,36,82,248,127,32,0,0,0,5,0,0,0,2,0,0,0,0,0,0,0,65,114,105,116,104,0,0,0,104,101,108,112,101,114,0,109,97,105,110,0,45,86,132,43,205,124,76,156,40,0,0,0,84,0,0,0,200,3,152,206,162,98,89,31,47,0,0,0,100,0,0,0,45,86,132,43,205,124,76,156,0,0,0,0,0,0,0,0,200,3,152,206,162,98,89,31,0,0,0,0,0,0,0,0
