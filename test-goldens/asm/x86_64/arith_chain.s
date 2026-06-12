
w_f9e6e6ef197c2b25:
  jmp .b0_0
.b0_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 48879
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_0
  mov [__lang_ds_high], r15
.ds_high_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 51966
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_2
  mov [__lang_ds_high], r15
.ds_high_2:
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  add rax, rcx
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_3
  mov [__lang_ds_high], r15
.ds_high_3:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 57005
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_4
  mov [__lang_ds_high], r15
.ds_high_4:
  sub r15, 8
  mov rcx, [r15]
  sub r15, 8
  mov rax, [r15]
  sub rax, rcx
  mov [r15], rax
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_5
  mov [__lang_ds_high], r15
.ds_high_5:
  jmp .endword_0
.endword_0:
  ret
