	.syntax unified
	.thumb
	.section .text
	.global __lang_trap
	.global __stack_overflow
	.extern __lang_trap
	.extern __stack_overflow
	.extern __lang_stack_limit
	.extern __lang_aperture_0_base
	.extern __lang_aperture_1_base

	.thumb_func
	.global w_1f5962a2ce9803c8
	.type w_1f5962a2ce9803c8, %function
w_1f5962a2ce9803c8:
	sub sp, sp, #24
	str lr, [sp, #20]
	ldr ip, =__lang_stack_limit
	cmp sp, ip
	bhs .Lsk0
	b __stack_overflow
.Lsk0:
	b .b0_0
.b0_0:
	movs r0, #42
	str r0, [r4]
	adds r4, r4, #4
	eors r0, r0
	str r0, [r4]
	adds r4, r4, #4
	push {r0, r1}
	ldr r1, =__lang_ds_high
	ldr r0, [r1]
	cmp r4, r0
	bls .ds_high_1
	str r4, [r1]
.ds_high_1:
	pop {r0, r1}
	subs r4, r4, #8
	ldrd r0, r1, [r4]
	strd r0, r1, [sp, #8]
	ldrd r0, r1, [sp, #8]
	strd r0, r1, [r4]
	adds r4, r4, #8
	push {r0, r1}
	ldr r1, =__lang_ds_high
	ldr r0, [r1]
	cmp r4, r0
	bls .ds_high_2
	str r4, [r1]
.ds_high_2:
	pop {r0, r1}
	b .endword_0
.endword_0:
	ldr lr, [sp, #20]
	add sp, sp, #24
	bx lr
	.section .lang.modinfo
	.byte 68,79,77,76,4,0,0,0,67,152,6,177,43,96,43,78,48,0,0,0,4,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,77,97,105,110,109,97,105,110,0,0,0,0,200,3,152,206,162,98,89,31,52,0,0,0,76,0,0,0,200,3,152,206,162,98,89,31,0,0,0,0,1,0,0,0
