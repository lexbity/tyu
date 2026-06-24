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
    .word __lang_hardfault + 1      @ NMI
    .word __lang_hardfault + 1      @ HardFault
.size _vectors, . - _vectors

@ -----------------------------------------------------------------
@ Shared semihosting helpers (local, not exported)
@ -----------------------------------------------------------------
.section .text, "x", %progbits

@ __lang_writec ( r0:byte -- )
@ Emit low byte of r0 via ARM semihosting SYS_WRITEC.
@ Preserves r4-r11 (AAPCS callee-save) AND r0, so callers emitting a run of
@ equal bytes (e.g. `movs r0, #0; bl __lang_writec` repeated) keep their value
@ across calls instead of seeing the clobbered SYS_WRITEC operation number.
__lang_writec:
    push {r0}
    mov r0, #0x03                  @ SYS_WRITEC
    mov r1, sp                     @ r1 = pointer to byte
    bkpt 0xAB
    pop {r0}                       @ restore caller's byte value
    bx lr

@ __lang_sys_exit ( r1:reason -- )
@ Terminate via ARM semihosting SYS_EXIT with reason code in r1.
@ For 32-bit ARM semihosting, SYS_EXIT takes the reason code DIRECTLY in r1 —
@ not a pointer to a block (that is the 64-bit form).  Passing a pointer makes
@ QEMU see an unrecognized reason and exit with status 1 instead of terminating
@ cleanly on ADP_Stopped_ApplicationExit.
@ Never returns.
__lang_sys_exit:
    movs r0, #0x18                 @ SYS_EXIT; r1 already holds the reason
    bkpt 0xAB
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
    @ Initialize V-once flag (BSS is .bss, zero-initialized by startup)
    ldr r0, =__lang_v_emitted
    movs r1, #0
    str r1, [r0]

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
@ Diagnostic D record emitter (shared)
@
@ Emits V (once) + framed D header via __lang_writec, then
@ falls through to __lang_fail_exit.
@
@ Caller must save caller-saved registers (r0-r3, r12) before
@ branching here, because __lang_writec clobbers them.
@
@ Input registers (preserved by __lang_writec):
@   r6 = ds_depth (u32, in slots)
@   r7 = word_hash low 32 bits
@   r8 = word_hash high 32 bits
@   r9 = trap_code (u32, low 16 bits used)
@   r10 = valid (0 or 1)
@   r11 = source_line (u32)
@ -----------------------------------------------------------------
emit_diag:
    @ Emit V version record at most once.
    ldr r0, =__lang_v_emitted
    ldr r0, [r0]
    cmp r0, #0
    bne emit_diag_header
    mov r0, #0x56                  @ 'V'
    bl __lang_writec
    mov r0, #1                     @ len = 1 (u16-le)
    bl __lang_writec
    mov r0, #0                     @ len high byte
    bl __lang_writec
    mov r0, #1                     @ version = 1
    bl __lang_writec
    ldr r0, =__lang_v_emitted
    movs r1, #1
    str r1, [r0]

emit_diag_header:
    @ D marker
    mov r0, #0x44                  @ 'D'
    bl __lang_writec

    @ Total payload length = 35 (u16-le, header-only, slot_count=0)
    mov r0, #35                    @ len low byte
    bl __lang_writec
    mov r0, #0                     @ len high byte
    bl __lang_writec

    @ Byte  0: DiagRecord.version = 1
    mov r0, #1
    bl __lang_writec

    @ Byte  1: origin = IN_GUEST (1)
    mov r0, #1
    bl __lang_writec

    @ Byte  2: valid = r10
    mov r0, r10
    bl __lang_writec

    @ Bytes 3-4: trap_code (u16-le) from r9
    mov r0, r9
    bl __lang_writec                 @ byte 0 (LSB)
    lsr r0, r9, #8
    bl __lang_writec                 @ byte 1 (MSB)

    @ Bytes 5-8: source_line (u32-le) from r11
    mov r0, r11
    bl __lang_writec
    lsr r0, r11, #8
    bl __lang_writec
    lsr r0, r11, #16
    bl __lang_writec
    lsr r0, r11, #24
    bl __lang_writec

    @ Bytes 9-16: word_hash (u64-le) from r7:r8 (low:high)
    mov r0, r7
    bl __lang_writec
    lsr r0, r7, #8
    bl __lang_writec
    lsr r0, r7, #16
    bl __lang_writec
    lsr r0, r7, #24
    bl __lang_writec
    mov r0, r8
    bl __lang_writec
    lsr r0, r8, #8
    bl __lang_writec
    lsr r0, r8, #16
    bl __lang_writec
    lsr r0, r8, #24
    bl __lang_writec

    @ Bytes 17-24: trap_pc (u64-le) = 0 (not available on header-only path)
    mov r0, #0
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec

    @ Bytes 25-28: ds_depth (u32-le) from r6
    mov r0, r6
    bl __lang_writec
    lsr r0, r6, #8
    bl __lang_writec
    lsr r0, r6, #16
    bl __lang_writec
    lsr r0, r6, #24
    bl __lang_writec

    @ Bytes 29-32: ds_declared = 0xFFFFFFFF (unknown / ⊤)
    mov r0, #0xFF
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec

    @ Bytes 33-34: slot_count = 0 (header-only)
    mov r0, #0
    bl __lang_writec
    bl __lang_writec

    b __lang_fail_exit

@ -----------------------------------------------------------------
@ __lang_hardfault — hardware fault entry (NMI / HardFault vectors)
@
@ Entered via vector table with exception frame on stack:
@   [sp+0x00]=r0, [sp+0x04]=r1, [sp+0x08]=r2, [sp+0x0C]=r3
@   [sp+0x10]=r12, [sp+0x14]=LR, [sp+0x18]=PC, [sp+0x1C]=xPSR
@
@ The faulting PC is at [sp + 0x18].  r0-r3, r12 are garbage
@ (saved from the faulting context).  valid=0.
@
@ AAPCS: on exception entry, the CPU is in Handler mode with
@ MSP (main stack pointer).  We must preserve r4-r11.
@ -----------------------------------------------------------------
.global __lang_hardfault
.type __lang_hardfault, %function
__lang_hardfault:
    @ Save callee-saved regs, including lr (r14).
    push {r4, r5, r6, r7, r8, r9, r10, r11, lr}

    @ Compute ds_depth from r4 (DS pointer, still valid).
    mov r6, r4
    ldr r0, =__lang_ds_base
    sub r6, r6, r0                  @ r6 = DS bytes used
    lsr r6, r6, #2                  @ r6 = ds_depth (slots, slot_bytes=4)

    @ Read faulting PC from exception frame at [sp + 9*4 + 0x18].
    @ After push {r4-r11, lr}, the stack has 9 saved regs (36 bytes).
    @ The original exception frame starts at current_sp + 36.
    ldr r0, [sp, #36 + 0x18]        @ r0 = faulting PC

    @ valid = 0, trap_code = 0, source_line = 0, word_hash = 0
    mov r7, #0                      @ word_hash low = 0
    mov r8, #0                      @ word_hash high = 0
    mov r9, #0                      @ trap_code = 0
    mov r10, #0                     @ valid = 0
    mov r11, #0                     @ source_line = 0
    @ trap_pc = r0 is NOT emitted by the header-only path.
    @ The host decoder reconstructs PC from the exception frame
    @ when valid=0 via the gdbstub escalation (Phase 14).

    @ Emit D header via shared routine.
    bl emit_diag
    @ emit_diag never returns (branches to __lang_fail_exit).

@ -----------------------------------------------------------------
@ __lang_trap — generic trap from compiled code
@
@ Entered via:  b __lang_trap
@ Register contract:
@   r0 = trap_code (set by compiler), r1/r2/r3 = undefined -> valid=0
@ -----------------------------------------------------------------
.global __lang_trap
.type __lang_trap, %function
__lang_trap:
    push {r4, r5, r6, r7, r8, r9, r10, r11, lr}

    @ Compute ds_depth from r4.
    mov r6, r4
    ldr r1, =__lang_ds_base
    sub r6, r6, r1
    lsr r6, r6, #2

    @ Save trap payload: r0 = trap_code, valid=0, line=0, word_hash=0
    mov r9, r0                      @ trap_code
    mov r10, #0                     @ valid = 0
    mov r11, #0                     @ source_line = 0
    mov r7, #0                      @ word_hash low = 0
    mov r8, #0                      @ word_hash high = 0

    bl emit_diag
    @ never returns

@ -----------------------------------------------------------------
@ __lang_trap_loc — trap with source location (debug_trap_loc=true)
@
@ Entered via:  b __lang_trap_loc
@ Register contract:
@   r0  = trap_code
@   r1  = valid (1)
@   r2  = source_line
@   r3  = word_hash low 32 bits
@   r12 = word_hash high 32 bits
@ -----------------------------------------------------------------
.global __lang_trap_loc
.type __lang_trap_loc, %function
__lang_trap_loc:
    push {r4, r5, r6, r7, r8, r9, r10, r11, lr}

    @ Save trap payload FIRST — the ds_depth computation below clobbers r1,
    @ which carries `valid` from the caller.  (Reading valid after the clobber
    @ produced the low byte of __lang_ds_base instead of 1.)
    mov r9, r0                      @ trap_code
    mov r10, r1                     @ valid (1)
    mov r11, r2                     @ source_line
    mov r7, r3                      @ word_hash low 32
    mov r8, r12                     @ word_hash high 32

    @ Compute ds_depth from r4 (r1 now free as scratch).
    mov r6, r4
    ldr r1, =__lang_ds_base
    sub r6, r6, r1
    lsr r6, r6, #2

    bl emit_diag
    @ never returns

@ -----------------------------------------------------------------
@ __stack_overflow — data-stack overflow detected at runtime
@
@ Entered from the emitted DS bounds check.  No registers set
@ for payload — valid=0, trap_code=10.
@ -----------------------------------------------------------------
.global __stack_overflow
.type __stack_overflow, %function
__stack_overflow:
    push {r4, r5, r6, r7, r8, r9, r10, r11, lr}

    @ Compute ds_depth from r4.
    mov r6, r4
    ldr r1, =__lang_ds_base
    sub r6, r6, r1
    lsr r6, r6, #2

    @ Stack overflow: trap_code=10, valid=0
    mov r9, #10                     @ trap_code = STACK_OVERFLOW
    mov r10, #0                     @ valid = 0
    mov r11, #0                     @ source_line = 0
    mov r7, #0                      @ word_hash low = 0
    mov r8, #0                      @ word_hash high = 0

    bl emit_diag
    @ never returns

@ -----------------------------------------------------------------
@ testio words — semihosting via shared snippet
@ -----------------------------------------------------------------
.include "../include/semihosting-arm.s"

@ -----------------------------------------------------------------
@ platform.gpio words — synthetic latch for smoke tests
@ -----------------------------------------------------------------
.global w_a6b1202e57aa7cc9
.type w_a6b1202e57aa7cc9, %function
w_a6b1202e57aa7cc9:
    subs r4, r4, #8
    ldr r0, =__lang_gpio_state
    movs r1, #0
    strb r1, [r0]
    bx lr

.global w_eb1d0a3c5e7c2e92
.type w_eb1d0a3c5e7c2e92, %function
w_eb1d0a3c5e7c2e92:
    subs r4, r4, #8
    ldr r0, =__lang_gpio_state
    ldrb r1, [r4, #4]
    strb r1, [r0]
    bx lr

.global w_034a1ff17acf93d3
.type w_034a1ff17acf93d3, %function
w_034a1ff17acf93d3:
    subs r4, r4, #4
    ldr r0, =__lang_gpio_state
    ldrb r1, [r0]
    str r1, [r4]
    bx lr

@ -----------------------------------------------------------------
@ platform.uart/time words — semihosting + monotonic stub
@ -----------------------------------------------------------------
.global w_6f29c37992fecaf8
.type w_6f29c37992fecaf8, %function
w_6f29c37992fecaf8:
    subs r4, r4, #4
    bx lr

.global w_38276faeb09bf91e
.type w_38276faeb09bf91e, %function
w_38276faeb09bf91e:
    subs r4, r4, #4
    ldr r0, [r4]
    bl __lang_writec
    bx lr

.global w_38126baeb0899648
.type w_38126baeb0899648, %function
w_38126baeb0899648:
    subs r4, r4, #4
    movs r0, #0
    str r0, [r4]
    adds r4, r4, #4
    movs r0, #0
    str r0, [r4]
    adds r4, r4, #4
    bx lr

.global w_6a5791a972f2fbd0
.type w_6a5791a972f2fbd0, %function
w_6a5791a972f2fbd0:
    ldr r0, =__lang_time_counter
    ldr r1, [r0]
    ldr r2, [r0, #4]
    adds r1, r1, #1
    adcs r2, r2, #0
    str r1, [r0]
    str r2, [r0, #4]
    str r1, [r4]
    adds r4, r4, #4
    str r2, [r4]
    adds r4, r4, #4
    bx lr

.global w_46f6f74f7859ca64
.type w_46f6f74f7859ca64, %function
w_46f6f74f7859ca64:
    b __lang_fail_exit

@ Note: `.modpack` / __lang_modpack_start/_end are provided by `modload.asm`
@ (assembled only when the module-loading feature is enabled), matching the
@ RISC-V runtime.  Defining them here too caused a duplicate-symbol link error.

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

.global __lang_v_emitted
__lang_v_emitted:
    .word 0

.global __lang_gpio_state
__lang_gpio_state:
    .word 0

.global __lang_time_counter
__lang_time_counter:
    .word 0
    .word 0

.section .data, "aw"
.global __lang_expected_abi_hash
__lang_expected_abi_hash:
    @ compute_abi_hash(ARCH_TAG_ARM=2, slot=4, word=32, MODINFO_VER=2) = 0xac34c6b7f7c80145, recipe v2
    .word 0xf7c80145
    .word 0xac34c6b7

    @ return to BSS for the native stack
    .section .bss, "aw", %nobits
    @ Guard zone below the usable stack: when a word prologue detects
    @ sp < __lang_stack_limit it branches to __stack_overflow, which then runs
    @ (emits its diagnostic) using this reserved headroom.
    .space 1024
    .global __lang_stack_limit
__lang_stack_limit:
    @ Native stack — 16 KB (grows downward, SP initialized from vector table).
    @ Sized to leave room in the 64 KB SRAM for the 16 KB data stack and the
    @ concurrency task pools; data-stack-heavy fixtures use the DS region, not
    @ deep native recursion.
    .space 16384
__stack_top:
