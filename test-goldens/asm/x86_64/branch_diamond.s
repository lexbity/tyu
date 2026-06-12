
w_f9e6e6ef197c2b25:
  jmp .b0_0
.b0_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 1
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_10
  mov [__lang_ds_high], r15
.ds_high_10:
  sub r15, 8
  mov rax, [r15]
  cmp rax, 0
  je .b0_2
  jmp .b0_1
.b0_1:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 48879
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_11
  mov [__lang_ds_high], r15
.ds_high_11:
  jmp .endword_0
.b0_2:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 51966
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_12
  mov [__lang_ds_high], r15
.ds_high_12:
  jmp .endword_0
.endword_0:
  ret
