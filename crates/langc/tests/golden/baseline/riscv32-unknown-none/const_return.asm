	.section .text
	.globl __lang_trap
	.globl __stack_overflow
	.extern __lang_trap
	.extern __stack_overflow
	.extern __lang_stack_limit
	.extern __lang_aperture_0_base

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
	li a0, 0x2a
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
	j .endword_0
.endword_0:
	lw ra, 28(sp)
	addi sp, sp, 32
	ret
	.section .lang.modinfo
	.byte 68,79,77,76,4,0,0,0,66,60,90,138,79,184,213,73,48,0,0,0,4,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,77,97,105,110,109,97,105,110,0,0,0,0,200,3,152,206,162,98,89,31,52,0,0,0,76,0,0,0,200,3,152,206,162,98,89,31,0,0,0,0,1,0,0,0
