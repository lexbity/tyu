
w_f9e6e6ef197c2b25:
  jmp .b0_0
.b0_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 0x1000
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_6
  mov [__lang_ds_high], r15
.ds_high_6:
  sub r15, 8
  mov rax, [r15]
  mov rax, qword [rax]
  mov [r15], rax
  add r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 0x2000
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_7
  mov [__lang_ds_high], r15
.ds_high_7:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 48879
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_8
  mov [__lang_ds_high], r15
.ds_high_8:
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  mov qword [rax], rcx
  sub r15, 8
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 0
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_9
  mov [__lang_ds_high], r15
.ds_high_9:
  jmp .endword_0
.endword_0:
  ret
