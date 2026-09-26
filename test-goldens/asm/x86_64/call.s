
w_f9e6e6ef197c2b25:
  jmp .b0_0
.b0_0:
  lea rax, [r15+8]
  cmp rax, r14
  ja __stack_overflow
  mov qword [r15], 0
  add r15, 8
  cmp r15, [__lang_ds_high]
  jna .ds_high_1
  mov [__lang_ds_high], r15
.ds_high_1:
  jmp .endword_0
.endword_0:
  ret
