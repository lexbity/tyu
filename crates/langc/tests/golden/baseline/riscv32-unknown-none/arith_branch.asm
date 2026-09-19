	.section .text
	.globl __lang_trap
	.globl __stack_overflow
	.extern __lang_trap
	.extern __stack_overflow
	.extern __lang_stack_limit
	.extern __lang_window_0_base

	.globl w_1f5962a2ce9803c8
	.type w_1f5962a2ce9803c8, @function
w_1f5962a2ce9803c8:
	addi sp, sp, -32
	sw ra, 28(sp)
.Laddr_load_1:
	auipc t0, %pcrel_hi(.Laddr_word_1)
	lw t0, %pcrel_lo(.Laddr_load_1)(t0)
	j .Laddr_after_1
	.balign 4
.Laddr_word_1:
	.word __lang_stack_limit
.Laddr_after_1:
	bgeu sp, t0, .Lsk0
	j __stack_overflow
.Lsk0:
	j .b0_0
.b0_0:
	li a0, 0xa
	sw a0, 0(s2)
	addi s2, s2, 4
	li a0, 0
	sw a0, 0(s2)
	addi s2, s2, 4
.Laddr_load_3:
	auipc t0, %pcrel_hi(.Laddr_word_3)
	lw t0, %pcrel_lo(.Laddr_load_3)(t0)
	j .Laddr_after_3
	.balign 4
.Laddr_word_3:
	.word __lang_ds_high
.Laddr_after_3:
	lw t1, 0(t0)
	bltu s2, t1, .ds_high_2
	sw s2, 0(t0)
.ds_high_2:
	li a0, 0x7
	sw a0, 0(s2)
	addi s2, s2, 4
	li a0, 0
	sw a0, 0(s2)
	addi s2, s2, 4
.Laddr_load_5:
	auipc t0, %pcrel_hi(.Laddr_word_5)
	lw t0, %pcrel_lo(.Laddr_load_5)(t0)
	j .Laddr_after_5
	.balign 4
.Laddr_word_5:
	.word __lang_ds_high
.Laddr_after_5:
	lw t1, 0(t0)
	bltu s2, t1, .ds_high_4
	sw s2, 0(t0)
.ds_high_4:
	addi s2, s2, -8
	lw a2, 0(s2)
	lw a3, 4(s2)
	addi s2, s2, -8
	lw a0, 0(s2)
	lw a1, 4(s2)
	add a0, a0, a2
	sltu a4, a0, a2
	add a1, a1, a3
	add a1, a1, a4
	sw a0, 0(s2)
	sw a1, 4(s2)
	addi s2, s2, 8
	li a0, 0x3
	sw a0, 0(s2)
	addi s2, s2, 4
	li a0, 0
	sw a0, 0(s2)
	addi s2, s2, 4
.Laddr_load_7:
	auipc t0, %pcrel_hi(.Laddr_word_7)
	lw t0, %pcrel_lo(.Laddr_load_7)(t0)
	j .Laddr_after_7
	.balign 4
.Laddr_word_7:
	.word __lang_ds_high
.Laddr_after_7:
	lw t1, 0(t0)
	bltu s2, t1, .ds_high_6
	sw s2, 0(t0)
.ds_high_6:
	addi s2, s2, -8
	lw a2, 0(s2)
	lw a3, 4(s2)
	addi s2, s2, -8
	lw a0, 0(s2)
	lw a1, 4(s2)
	mul a0, a0, a2
	li a1, 0
	sw a0, 0(s2)
	sw a1, 4(s2)
	addi s2, s2, 8
	addi s2, s2, -8
	lw a0, 0(s2)
	lw a1, 4(s2)
	sw a0, 8(sp)
	sw a1, 12(sp)
	lw a0, 8(sp)
	lw a1, 12(sp)
	sw a0, 0(s2)
	sw a1, 4(s2)
	addi s2, s2, 8
.Laddr_load_9:
	auipc t0, %pcrel_hi(.Laddr_word_9)
	lw t0, %pcrel_lo(.Laddr_load_9)(t0)
	j .Laddr_after_9
	.balign 4
.Laddr_word_9:
	.word __lang_ds_high
.Laddr_after_9:
	lw t1, 0(t0)
	bltu s2, t1, .ds_high_8
	sw s2, 0(t0)
.ds_high_8:
	j .endword_0
.endword_0:
	lw ra, 28(sp)
	addi sp, sp, 32
	ret
	.section .lang.modinfo
	.byte 68,79,77,76,3,0,0,0,133,189,48,228,163,52,221,246,32,0,0,0,4,0,0,0,1,0,0,0,0,0,0,0,77,97,105,110,109,97,105,110,0,0,0,0,200,3,152,206,162,98,89,31,36,0,0,0,60,0,0,0,200,3,152,206,162,98,89,31,0,0,0,0,2,0,0,0
