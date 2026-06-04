@ ARM Cortex-M3 bare-metal runtime (lm3s6965evb QEMU machine)
@
@ Vector table at FLASH 0x0; code and read-only data in FLASH;
@ BSS (DS, high-water, native stack) in SRAM at 0x20000000.
@
@ ABI contract (abi-contract 4.4.1 / 4.4.2):
@   DS pointer = r4 (upward-growing: push = adds, pop = subs)
@   slot_bytes = 4
@   DS limit   = r5

.syntax unified
.thumb

@ -----------------------------------------------------------------
@ Vector table — placed at FLASH origin by link.ld
@ -----------------------------------------------------------------
.section .vectors, "a", %progbits
.type _vectors, %object
_vectors:
    .word __stack_top               @ initial SP
    .word __lang_start + 1          @ Reset_Handler (Thumb bit)
    .word __lang_trap + 1           @ NMI
    .word __lang_trap + 1           @ HardFault
.size _vectors, . - _vectors

@ -----------------------------------------------------------------
@ Shared semihosting helpers (local, not exported)
@ -----------------------------------------------------------------
.section .text, "x", %progbits

@ __lang_writec ( r0:byte -- )
@ Emit low byte of r0 via ARM semihosting SYS_WRITEC.
@ Preserves r4-r11 (AAPCS callee-save).
__lang_writec:
    push {r0}
    mov r0, #0x03                  @ SYS_WRITEC
    mov r1, sp                     @ r1 = pointer to byte
    bkpt 0xAB
    add sp, sp, #4
    bx lr

@ __lang_sys_exit ( r1:reason -- )
@ Terminate via ARM semihosting SYS_EXIT with reason code in r1.
@ Never returns.
__lang_sys_exit:
    push {r1}
    mov r1, sp
    movs r0, #0x18                 @ SYS_EXIT
    bkpt 0xAB
    add sp, sp, #4
    bkpt #0                        @ should not reach here

@ __lang_fail_exit ( -- )
@ Terminate with ADP_Stopped_ApplicationExit (0x20026).
@ Never returns.
__lang_fail_exit:
    ldr r1, =0x20026
    b __lang_sys_exit

@ -----------------------------------------------------------------
@ Entry point
@ -----------------------------------------------------------------

.global __lang_start
.type __lang_start, %function
__lang_start:
    @ DS pointer r4 = __lang_ds_base (low address, grows upward)
    ldr r4, =__lang_ds_base
    @ DS limit r5 = __lang_ds_limit (exclusive upper bound)
    ldr r5, =__lang_ds_limit
    @ Initialize high-water to DS base
    ldr r0, =__lang_ds_high
    str r4, [r0]

    bl w_1f5962a2ce9803c8          @ call main ( -- i64 )

    @ Pop exit code from DS — main returns ( -- i64 ), i64 = two 4-byte slots
    subs r4, r4, #8
    ldr r0, [r4]                   @ r0 = low 32 bits of exit code (discarded)

    @ Emit high-water: 'H' + u32-le (peak DS depth in slots)
    ldr r1, =__lang_ds_high
    ldr r0, [r1]                   @ r0 = max DS pointer
    ldr r1, =__lang_ds_base
    sub r0, r0, r1                 @ r0 = bytes used
    lsr r0, r0, #2                 @ r0 = slots (slot_bytes = 4)
    mov r6, r0                     @ save slot count in r6

    mov r0, #0x48                  @ 'H'
    bl __lang_writec
    mov r0, r6                     @ byte 0 (LSB)
    bl __lang_writec
    lsr r0, r6, #8                 @ byte 1
    bl __lang_writec
    lsr r0, r6, #16                @ byte 2
    bl __lang_writec
    lsr r0, r6, #24                @ byte 3 (MSB)
    bl __lang_writec

    b __lang_fail_exit

@ -----------------------------------------------------------------
@ Trap handlers
@ -----------------------------------------------------------------

.global __lang_trap
.type __lang_trap, %function
.global __lang_trap_loc
.type __lang_trap_loc, %function
.global __stack_overflow
.type __stack_overflow, %function
__lang_trap:
__lang_trap_loc:
__stack_overflow:
    b __lang_fail_exit

@ -----------------------------------------------------------------
@ testio words — semihosting via shared snippet
@ -----------------------------------------------------------------
.include "../include/semihosting-arm.s"

@ -----------------------------------------------------------------
@ Module modpack section (S2 Phase 14)
@ Embedded .lmod images live in their own section, each prefixed
@ with a u32 length. Scanned by loader [start .. end).
@ -----------------------------------------------------------------
.section .modpack, "aw", %nobits
.global __lang_modpack_start
__lang_modpack_start:
.global __lang_modpack_end
__lang_modpack_end:

@ -----------------------------------------------------------------
@ BSS — DS region, high-water, native stack
@ -----------------------------------------------------------------
.section .bss, "aw", %nobits

    @ Data stack — 16 KB. r4 starts at __lang_ds_base (low address)
    @ and grows upward. r5 = __lang_ds_limit (upper bound, exclusive).
.global __lang_ds_base
__lang_ds_base:
    .space 16384
.global __lang_ds_limit
__lang_ds_limit:

.global __lang_ds_high
__lang_ds_high:
    .word 0

.global __lang_expected_abi_hash
__lang_expected_abi_hash:
    .word 0x53048547
    .word 0x0445187d

    @ Native stack — 32 KB (grows downward, SP initialized from vector table)
    .space 32768
__stack_top:
