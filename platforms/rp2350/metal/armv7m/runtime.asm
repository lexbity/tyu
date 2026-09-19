@ RP2350 ARMv7-M metal runtime — board pack fork (P8).
@
@ The generic ARM runtime stays in runtime/armv7m-unknown-none/. This copy
@ binds the portable surface to RP2350 hardware and brings the board up from
@ the bootrom handoff state:
@   - XOSC (12 MHz crystal), PLL_SYS -> clk_sys = clk_peri = 150 MHz
@   - clk_ref = XOSC, TIMER0 tick generator = 1 us            (DS2 8.5)
@   - resets released for every peripheral this runtime drives
@   - GPIO via IO_BANK0 + PADS_BANK0 + SIO; UART via UART0; time via TIMER0
@   - platform.mem.region words: reference bump allocator (P7, metal.trust)
@
@ Every device constant is transcribed from RP-008373-DS-2
@ (devdocs/HW-datasheets); see platforms/rp2350/docs/descriptor-notes.md.

.syntax unified
.thumb

.equ XOSC_BASE,       0x40048000   @ DS2 Table 13 (APB address map)
.equ CLOCKS_BASE,     0x40010000
.equ PLL_SYS_BASE,    0x40050000
.equ RESETS_BASE,     0x40020000
.equ IO_BANK0_BASE,   0x40028000
.equ PADS_BANK0_BASE, 0x40038000
.equ UART0_BASE,      0x40070000
.equ TIMER0_BASE,     0x400b0000
.equ SIO_BASE,        0xd0000000
.equ TICKS_BASE,      0x40108000
.equ PPB_VTOR,        0xe000ed08   @ Cortex-M33 Vector Table Offset Register

@ XOSC (DS2 8.2): FREQ_RANGE 1-15 MHz code 0xaa0 in [11:0], ENABLE key
@ 0xfff in [23:12]; STATUS.STABLE is bit 31; STARTUP.DELAY in [13:0],
@ counted in 256-xtal-cycle units.
.equ XOSC_CTRL_VAL,    0x0fff0aa0
.equ XOSC_STARTUP_VAL, 64         @ 64 * 256 xtal cycles (~1.4 ms @ 12 MHz)

@ RESETS bit positions (DS2 7.5.2): UART0=26, TIMER0=23, PLL_SYS=14,
@ PADS_BANK0=9, IO_BANK0=6.  RESET at +0x00, RESET_DONE at +0x08.
.equ RESETS_MASK,     ((1<<26)|(1<<23)|(1<<14)|(1<<9)|(1<<6))

@ CLOCKS offsets (DS2 8.1.7).  CLK_SYS_CTRL: SRC[0] 1=aux, AUXSRC[5:3]
@ 0=pll_sys.  CLK_PERI_CTRL: ENABLE bit 11, AUXSRC[7:5] 0=clk_sys.
@ CLK_REF_CTRL: SRC[1:0] 2=xosc_clksrc.  The *_SELECTED registers report
@ the glitchless mux position one-hot and must be polled after a switch.
.equ CLK_REF_CTRL_OFF,     0x30
.equ CLK_REF_SELECTED_OFF, 0x38
.equ CLK_SYS_CTRL_OFF,     0x3c
.equ CLK_SYS_SELECTED_OFF, 0x44
.equ CLK_PERI_CTRL_OFF,    0x48
.equ CLK_SYS_CTRL_AUX_PLL, 0x1    @ SRC=1 (aux), AUXSRC=0 (pll_sys)
.equ CLK_PERI_CTRL_ENABLE, 0x800  @ ENABLE, AUXSRC=0 (clk_sys)
.equ CLK_REF_CTRL_XOSC,    0x2    @ SRC=2 (xosc_clksrc)

@ PLL_SYS (DS2 8.6): 12 MHz / 1 * 125 = 1500 MHz VCO, / 5 / 2 = 150 MHz.
@ CS.REFDIV [5:0]; PWR bits PD=0, DSMPD=2, POSTDIVPD=3, VCOPD=5;
@ FBDIV_INT at +0x08; PRIM at +0x0c with POSTDIV1 [18:16], POSTDIV2 [14:12];
@ CS.LOCK is bit 31.
.equ PLL_CS_REFDIV_1,   0x1
.equ PLL_PWR_ALL_OFF,   0x2d      @ PD | DSMPD | POSTDIVPD | VCOPD
.equ PLL_PWR_VCO_ONLY,  0x20      @ VCOPD
.equ PLL_FBDIV_125,     125
.equ PLL_PRIM_DIV5_2,   0x52000   @ POSTDIV1=5 | POSTDIV2=2 << 12

@ Tick generators run from clk_ref (DS2 8.5): 12 cycles of the 12 MHz XOSC
@ give the 1 us tick TIMER0 defaults to.  TIMER0_CTRL at +0x18 (ENABLE bit 0,
@ RUNNING bit 1), TIMER0_CYCLES at +0x1c.
.equ TICKS_TIMER0_CTRL_OFF,   0x18
.equ TICKS_TIMER0_CYCLES_OFF, 0x1c

@ IO_BANK0 (DS2 9.11): each pin is { STATUS +8n, CTRL +8n+4 }.
.equ IO_CTRL_OFF,     0x004       @ GPIO0_CTRL (UART0 TX function select)
.equ FUNCSEL_UART0,   0x02
.equ FUNCSEL_SIO,     0x05

@ SIO (DS2 3.1.11): RP2350 interleaves the GPIO_HI bank, so the SET/CLR/OE
@ aliases sit 4 bytes higher than on RP2040.
.equ SIO_GPIO_IN,      0x004
.equ SIO_GPIO_OUT_SET, 0x018
.equ SIO_GPIO_OUT_CLR, 0x020
.equ SIO_GPIO_OE_SET,  0x038
.equ SIO_GPIO_OE_CLR,  0x040

@ PADS_BANK0 (DS2 9.11.3): IE bit 6, ISO bit 8.
.equ PAD_IE,          0x040
.equ PAD_ISO,         0x100

@ UART0 at 115200 baud from clk_peri = 150 MHz (DS2 12.1):
@ bauddiv = 150e6 / (16 * 115200) = 81.38 -> IBRD=81, FBRD=round(0.38*64)=24.
.equ UART_IBRD_150MHZ_115200, 81
.equ UART_FBRD_150MHZ_115200, 24

.section .vectors, "a", %progbits
.type _vectors, %object
.globl __lang_vectors
__lang_vectors:
_vectors:
    .word __stack_top                @ 0  initial SP (linker script: SRAM top)
    .word __lang_start + 1           @ 1  Reset
    .rept 14                         @ 2..15  NMI..SysTick -> trap dump
    .word __lang_hardfault
    .endr
    .rept 52                         @ 16..67 IRQ0..IRQ51 (DS2 Table 95)
    .word __lang_hardfault
    .endr
.size _vectors, . - _vectors

.text

.global __lang_writec
.type __lang_writec, %function
__lang_writec:
    push {r1, r2, r3, lr}
    bl __lang_uart_putc
    pop {r1, r2, r3, lr}
    bx lr

.type __lang_uart_putc, %function
__lang_uart_putc:
    ldr r1, =UART0_BASE
1:
    ldr r2, [r1, #0x18]
    tst r2, #0x20                  @ FR.TXFF
    bne 1b
    str r0, [r1, #0x00]
    bx lr

@ ---------------------------------------------------------------------------
@ Board bring-up (runs from the bootrom handoff clock state).
@ Preserves r4/r5 (DS pointer / limit).
@ ---------------------------------------------------------------------------
.type __lang_clocks_init, %function
__lang_clocks_init:
    push {r4, r5, lr}

    @ XOSC on and stable (DS2 8.2)
    ldr r4, =XOSC_BASE
    ldr r0, =XOSC_CTRL_VAL
    str r0, [r4, #0x00]            @ CTRL
    ldr r0, =XOSC_STARTUP_VAL
    str r0, [r4, #0x0c]            @ STARTUP
1:
    ldr r0, [r4, #0x04]            @ STATUS
    lsrs r0, r0, #31               @ STABLE
    beq 1b

    @ Release resets for UART0, TIMER0, PLL_SYS, PADS_BANK0, IO_BANK0
    ldr r4, =RESETS_BASE
    ldr r0, [r4, #0x00]            @ RESET
    ldr r1, =RESETS_MASK
    bics r0, r0, r1
    str r0, [r4, #0x00]
2:
    ldr r0, [r4, #0x08]            @ RESET_DONE
    ldr r1, =RESETS_MASK
    ands r0, r0, r1
    cmp r0, r1
    bne 2b

    @ PLL_SYS -> 150 MHz (DS2 8.6)
    ldr r4, =PLL_SYS_BASE
    ldr r0, =PLL_CS_REFDIV_1
    str r0, [r4, #0x00]            @ CS: REFDIV = 1
    ldr r0, =PLL_PWR_ALL_OFF
    str r0, [r4, #0x04]            @ PWR: everything off
    ldr r0, =PLL_FBDIV_125
    str r0, [r4, #0x08]            @ FBDIV_INT = 125
    ldr r0, =PLL_PWR_VCO_ONLY
    str r0, [r4, #0x04]            @ PWR: VCO only
3:
    ldr r0, [r4, #0x00]            @ CS
    lsrs r0, r0, #31               @ LOCK
    beq 3b
    ldr r0, =PLL_PRIM_DIV5_2
    str r0, [r4, #0x0c]            @ PRIM: /5, /2
    movs r0, #0
    str r0, [r4, #0x04]            @ PWR: outputs on

    @ clk_sys -> aux(PLL_SYS); poll the glitchless mux until settled
    ldr r4, =CLOCKS_BASE
    ldr r0, =CLK_SYS_CTRL_AUX_PLL
    str r0, [r4, #CLK_SYS_CTRL_OFF]
4:
    ldr r0, [r4, #CLK_SYS_SELECTED_OFF]
    cmp r0, #2                     @ one-hot bit 1 = aux position
    bne 4b

    @ clk_peri <- clk_sys (drives UART0)
    ldr r0, =CLK_PERI_CTRL_ENABLE
    str r0, [r4, #CLK_PERI_CTRL_OFF]

    @ clk_ref -> XOSC (drives the tick generators)
    ldr r0, =CLK_REF_CTRL_XOSC
    str r0, [r4, #CLK_REF_CTRL_OFF]
5:
    ldr r0, [r4, #CLK_REF_SELECTED_OFF]
    cmp r0, #4                     @ one-hot bit 2 = xosc position
    bne 5b

    @ TIMER0 tick: 12 cycles of the 12 MHz XOSC = 1 us
    ldr r4, =TICKS_BASE
    movs r0, #0
    str r0, [r4, #TICKS_TIMER0_CTRL_OFF]
    movs r0, #12
    str r0, [r4, #TICKS_TIMER0_CYCLES_OFF]
    movs r0, #1
    str r0, [r4, #TICKS_TIMER0_CTRL_OFF]
6:
    ldr r0, [r4, #TICKS_TIMER0_CTRL_OFF]
    lsrs r0, r0, #1                @ RUNNING
    bcc 6b

    pop {r4, r5, pc}
.ltorg

.type __lang_uart_init, %function
__lang_uart_init:
    push {r1, r2, r3, r4, lr}

    @ GPIO0/1 pads: input enable on, release isolation (DS2 9.11.3)
    ldr r4, =PADS_BANK0_BASE
    ldr r2, [r4, #0x00]            @ GPIO0 pad
    orr r2, r2, #PAD_IE
    bic r2, r2, #PAD_ISO
    str r2, [r4, #0x00]
    ldr r2, [r4, #0x04]            @ GPIO1 pad
    orr r2, r2, #PAD_IE
    bic r2, r2, #PAD_ISO
    str r2, [r4, #0x04]

    @ GPIO0/1 -> UART0 function (CTRL at +8n+4, DS2 9.11)
    ldr r4, =IO_BANK0_BASE
    ldr r2, =FUNCSEL_UART0
    str r2, [r4, #IO_CTRL_OFF]     @ GPIO0_CTRL
    str r2, [r4, #0x0c]            @ GPIO1_CTRL

    @ 115200 8N1, FIFOs on, from clk_peri = 150 MHz
    ldr r4, =UART0_BASE
    movs r2, #0
    str r2, [r4, #0x30]            @ CR: disable while configuring
    ldr r2, =0x7ff
    str r2, [r4, #0x44]            @ ICR: clear pending interrupts
    ldr r2, =UART_IBRD_150MHZ_115200
    str r2, [r4, #0x24]            @ IBRD
    ldr r2, =UART_FBRD_150MHZ_115200
    str r2, [r4, #0x28]            @ FBRD
    ldr r2, =0x70                  @ LCR_H: 8N1, FIFO enable
    str r2, [r4, #0x2c]
    ldr r2, =0x301                 @ CR: UARTEN | TXE | RXE
    str r2, [r4, #0x30]

    pop {r1, r2, r3, r4, pc}
.ltorg

.type __lang_fail_exit, %function
__lang_fail_exit:
    b .

.global __lang_start
.type __lang_start, %function
__lang_start:
    @ data stack (DS) init — r4/r5 are the compiled-code DS convention
    ldr r4, =__lang_ds_base
    ldr r5, =__lang_ds_limit
    ldr r0, =__lang_ds_high
    str r4, [r0]
    @ V-once flag reset (BSS is zeroed just below, belt and braces for the
    @ dynamic path which re-enters after the zero loop has run)
    ldr r0, =__lang_v_emitted
    movs r1, #0
    str r1, [r0]

    @ route exceptions/IRQs to this runtime's vector table
    ldr r0, =__lang_vectors
    ldr r1, =PPB_VTOR
    str r0, [r1]

    @ zero .bss: on real silicon SRAM content is not guaranteed across
    @ reset, and the DS high-water marks and the region allocator rely on
    @ zero-initialized statics.
    ldr r0, =__bss_start
    ldr r1, =__bss_end
0:
    cmp r0, r1
    bhs 9f
    movs r2, #0
    str r2, [r0], #4
    b 0b
9:

    bl __lang_clocks_init
    bl __lang_uart_init
    cpsie i

    bl __lang_entry                @ static: call main; dynamic: load lmod then run main

.global __lang_after_main
.type __lang_after_main, %function
__lang_after_main:

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

@ -----------------------------------------------------------------
@ platform.mem.region words — reference bump allocator, ported from
@ runtime/armv7m-unknown-none/runtime.asm (P7, D-6 metal.trust).
@ -----------------------------------------------------------------
@ platform.mem.region words — reference bump allocator (P7, D-6 metal.trust)
@
@ Model (matches the x86 hosted codegen allocator): a static arena
@ `__region_arena` (4096 bytes) in BSS with 16 slots of { base, size, off }.
@ region-create carves a chunk from the arena and records it in a free slot;
@ region-alloc bumps `off` within the slot; region-reset zeroes `off`;
@ region-destroy frees the slot.
@
@ DS convention (abi-contract): r4 = DS pointer, slot_bytes = 4, upward.
@   pop i64  = subs r4,#8; ldrd r0,r1,[r4]
@   push i64 = strd r0,r1,[r4]; adds r4,#8
@ Trap: r0 = trap_code; b __lang_trap.
@ Exhaustion (no free slot / arena overrun / region full) raises
@ REGION_EXHAUSTED (26).  Register discipline: only r0-r3 and r12 are used
@ (caller-saved); r5-r11 preserved (AAPCS), matching the other runtime words.
@ -----------------------------------------------------------------
.global w_7a5f795caa045668
.type w_7a5f795caa045668, %function
w_7a5f795caa045668:
    subs r4, r4, #8
    ldrd r0, r1, [r4]          @ r0 = size (low)
    cmp r0, #0
    bne 1f
    movs r0, #23               @ UNREACHABLE: zero-size region
    b __lang_trap
1:
    @ align size to 8
    adds r0, r0, #7
    bic r0, r0, #7
    @ find a free slot: scan __region_size[0..15] for 0
    movs r1, #0                @ slot index
    ldr r2, =__region_size
2:
    cmp r1, #16
    bge 3f                     @ none free -> use __region_next
    ldr r3, [r2, r1, lsl #2]
    cmp r3, #0
    beq 4f                     @ free slot found
    adds r1, r1, #1
    b 2b
3:
    @ use __region_next if under 16, else exhausted
    ldr r2, =__region_next
    ldr r1, [r2]
    cmp r1, #16
    bge 8f
    ldr r3, [r2]
    adds r3, r3, #1
    str r3, [r2]
4:
    @ r1 = slot, r0 = aligned size
    @ carve from arena: base = __region_arena + __region_used
    ldr r2, =__region_used
    ldr r3, [r2]               @ used
    ldr r12, =__region_arena
    add r12, r12, r3           @ base
    add r3, r3, r0             @ new used
    @ end of this slot = base + size; must fit in arena
    add r12, r12, r0           @ r12 = base + size (end)
    ldr r2, =__region_arena_end
    cmp r12, r2
    bgt 8f                     @ arena overrun -> exhausted
    sub r12, r12, r0           @ r12 = base again
    ldr r2, =__region_used
    str r3, [r2]               @ used = new used
    @ record slot: base, size, off=0
    ldr r2, =__region_base
    str r12, [r2, r1, lsl #2]
    ldr r2, =__region_size
    str r0, [r2, r1, lsl #2]
    ldr r2, =__region_off
    movs r3, #0
    str r3, [r2, r1, lsl #2]
    @ return Region handle = slot index
    mov r0, r1
    movs r1, #0                @ high word
    strd r0, r1, [r4]
    adds r4, r4, #8
    bx lr
8:
    movs r0, #26               @ REGION_EXHAUSTED
    b __lang_trap

.global w_00433c33168e6701
.type w_00433c33168e6701, %function
w_00433c33168e6701:
    subs r4, r4, #8
    ldrd r2, r3, [r4]          @ r2 = usize (size, low)
    subs r4, r4, #8
    ldrd r0, r1, [r4]          @ r0 = Region handle
    cmp r0, #16
    bge 9f                     @ bad handle -> UNREACHABLE
    ldr r3, =__region_size
    ldr r3, [r3, r0, lsl #2]   @ slot size
    cmp r3, #0
    beq 9f                     @ dead slot -> UNREACHABLE
    @ r3 = slot size; align request to 8
    adds r2, r2, #7
    bic r2, r2, #7
    @ off + size <= slot_size ?
    ldr r12, =__region_off
    ldr r1, [r12, r0, lsl #2]  @ off
    add r1, r1, r2             @ new off
    cmp r1, r3
    bgt 8f                     @ region full -> REGION_EXHAUSTED
    str r1, [r12, r0, lsl #2]  @ off = new off
    @ ptr = base + old_off
    ldr r12, =__region_base
    ldr r3, [r12, r0, lsl #2]  @ base
    ldr r12, =__region_off
    ldr r1, [r12, r0, lsl #2]  @ new off
    sub r1, r1, r2             @ old off
    add r0, r3, r1             @ ptr
    movs r1, #0
    strd r0, r1, [r4]
    adds r4, r4, #8
    bx lr
8:
    movs r0, #26               @ REGION_EXHAUSTED
    b __lang_trap
9:
    movs r0, #23               @ UNREACHABLE
    b __lang_trap

.global w_a52160bb1e22438b
.type w_a52160bb1e22438b, %function
w_a52160bb1e22438b:
    subs r4, r4, #8
    ldrd r0, r1, [r4]
    cmp r0, #16
    bge 9f
    ldr r2, =__region_size
    ldr r3, [r2, r0, lsl #2]
    cmp r3, #0
    beq 9f
    ldr r2, =__region_off
    movs r3, #0
    str r3, [r2, r0, lsl #2]
    bx lr
9:
    movs r0, #23
    b __lang_trap

.global w_3dc921382ce34c3e
.type w_3dc921382ce34c3e, %function
w_3dc921382ce34c3e:
    subs r4, r4, #8
    ldrd r0, r1, [r4]
    cmp r0, #16
    bge 9f
    ldr r2, =__region_size
    ldr r3, [r2, r0, lsl #2]
    cmp r3, #0
    beq 9f
    movs r3, #0
    ldr r2, =__region_base
    str r3, [r2, r0, lsl #2]
    ldr r2, =__region_size
    str r3, [r2, r0, lsl #2]
    ldr r2, =__region_off
    str r3, [r2, r0, lsl #2]
    bx lr
9:
    movs r0, #23
    b __lang_trap

@ Note: `.modpack` / __lang_modpack_start/_end are provided by `modload.asm`
@ (assembled only when the module-loading feature is enabled), matching the
@ RISC-V runtime.  Defining them here too caused a duplicate-symbol link error.

@ -----------------------------------------------------------------
@ BSS — DS region, high-water, native stack
@ -----------------------------------------------------------------

.global __region_arena
__region_arena:
    .space 208
.global __region_arena_end
__region_arena_end:
.global __region_next
__region_next:
    .word 0
.global __region_used
__region_used:
    .word 0
.global __region_base
__region_base:
    .space 64
.global __region_size
__region_size:
    .space 64
.global __region_off
__region_off:
    .space 64


@ -----------------------------------------------------------------
@ testio words — UART-backed RP2350 implementation (metal.trust)
@ -----------------------------------------------------------------
.global w_accb676a903a06d9
.type w_accb676a903a06d9, %function
w_accb676a903a06d9:
    subs r4, r4, #8
    ldr r0, [r4]
    bl __lang_writec
    bx lr

.global w_eb06855547211672
.type w_eb06855547211672, %function
w_eb06855547211672:
    subs r4, r4, #4
    ldr r2, [r4]
    ldr r3, [r2]
    adds r2, r2, #8
    cbz r3, 1f
0:
    ldrb r0, [r2]
    bl __lang_writec
    adds r2, r2, #1
    subs r3, r3, #1
    bne 0b
1:
    bx lr

.global w_f91ca4f233247b4d
.type w_f91ca4f233247b4d, %function
w_f91ca4f233247b4d:
    subs r4, r4, #8
    ldr r0, [r4]
    b __lang_fail_exit

.global w_a6b1202e57aa7cc9
.type w_a6b1202e57aa7cc9, %function
w_a6b1202e57aa7cc9:
    subs r4, r4, #8
    ldr r0, [r4]
    bl __lang_gpio_init
    bx lr

.global w_eb1d0a3c5e7c2e92
.type w_eb1d0a3c5e7c2e92, %function
w_eb1d0a3c5e7c2e92:
    subs r4, r4, #8
    ldr r0, [r4]
    ldr r1, [r4, #4]
    bl __lang_gpio_write
    bx lr

.global w_034a1ff17acf93d3
.type w_034a1ff17acf93d3, %function
w_034a1ff17acf93d3:
    subs r4, r4, #4
    ldr r0, [r4]
    bl __lang_gpio_read
    str r0, [r4]
    adds r4, r4, #4
    bx lr

.type __lang_gpio_init, %function
__lang_gpio_init:
    push {r1, r2, r3, lr}
    @ r0 = pin, r1 = mode (0 = input, else output)
    @ pad: input enable on, isolation released
    ldr r2, =PADS_BANK0_BASE
    add r3, r2, r0, lsl #2
    ldr r2, [r3]
    orr r2, r2, #PAD_IE
    bic r2, r2, #PAD_ISO
    str r2, [r3]
    @ funcsel -> SIO (CTRL at +8n+4)
    ldr r2, =IO_BANK0_BASE
    add r3, r2, r0, lsl #3
    add r3, r3, #IO_CTRL_OFF
    ldr r2, =FUNCSEL_SIO
    str r2, [r3]
    @ output enable via the SIO OE aliases (RP2350 offsets)
    ldr r2, =SIO_BASE
    movs r3, #1
    lsls r3, r3, r0
    cmp r1, #0
    beq 1f
    str r3, [r2, #SIO_GPIO_OE_SET]
    b 2f
1:  str r3, [r2, #SIO_GPIO_OE_CLR]
2:  pop {r1, r2, r3, lr}
    bx lr

.type __lang_gpio_write, %function
__lang_gpio_write:
    push {r2, r3, lr}
    ldr r2, =SIO_BASE
    movs r3, #1
    lsls r3, r3, r0
    cmp r1, #0
    beq 1f
    str r3, [r2, #SIO_GPIO_OUT_SET]
    b 2f
1:  str r3, [r2, #SIO_GPIO_OUT_CLR]
2:  pop {r2, r3, lr}
    bx lr

.type __lang_gpio_read, %function
__lang_gpio_read:
    push {r1, r2, r3, lr}
    ldr r1, =SIO_BASE
    ldr r2, [r1, #SIO_GPIO_IN]
    movs r3, #1
    lsls r3, r3, r0
    ands r0, r2, r3
    cmp r0, #0
    it ne
    movne r0, #1
    it eq
    moveq r0, #0
    pop {r1, r2, r3, lr}
    bx lr

.global w_6f29c37992fecaf8
.type w_6f29c37992fecaf8, %function
w_6f29c37992fecaf8:
    subs r4, r4, #4
    bl __lang_uart_init
    bx lr

.global w_38276faeb09bf91e
.type w_38276faeb09bf91e, %function
w_38276faeb09bf91e:
    subs r4, r4, #4
    ldr r0, [r4]
    bl __lang_uart_putc
    bx lr

.global w_38126baeb0899648
.type w_38126baeb0899648, %function
w_38126baeb0899648:
    subs r4, r4, #4
    ldr r1, =UART0_BASE
1:  ldr r0, [r1, #0x18]
    tst r0, #0x10                  @ FR.RXFE
    bne 1b
    ldr r0, [r1, #0x00]
    str r0, [r4]
    adds r4, r4, #4
    movs r0, #1
    str r0, [r4]
    adds r4, r4, #4
    bx lr

.global w_6a5791a972f2fbd0
.type w_6a5791a972f2fbd0, %function
w_6a5791a972f2fbd0:
    @ 64-bit microsecond time from TIMER0 (1 us tick configured at start).
    @ TIMEHR only latches reliably via the TIMELR read, so read HR, LR, HR
    @ and retry until the two high words agree (DS2 12.8).
    ldr r0, =TIMER0_BASE
1:  ldr r1, [r0, #0x08]            @ TIMEHR
    ldr r2, [r0, #0x0c]            @ TIMELR (latches TIMEHR)
    ldr r3, [r0, #0x08]            @ TIMEHR again
    cmp r1, r3
    bne 1b
    str r2, [r4]                   @ low word
    adds r4, r4, #4
    str r3, [r4]                   @ high word
    adds r4, r4, #4
    bx lr

.global w_46f6f74f7859ca64
.type w_46f6f74f7859ca64, %function
w_46f6f74f7859ca64:
    @ platform.time.reboot: the RP2350 reboot path (REBOOT/RESCFG) is not
    @ modeled by this runtime; park instead of silently returning.
    b .

.section .bss, "aw", %nobits
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

@ Guard zone below the overflow threshold (__lang_stack_limit): when a word
@ prologue detects sp < __lang_stack_limit it branches to __stack_overflow,
@ which runs using this reserved headroom.  The native stack itself lives at
@ the top of SRAM (__stack_top, from the linker script).
    .space 1024
.global __lang_stack_limit
__lang_stack_limit:

.section .data, "aw"
.global __lang_expected_abi_hash
__lang_expected_abi_hash:
    @ compute_abi_hash(ARCH_TAG_ARM=2, slot=4, word=32, MODINFO_VER=4) = 0x4e2b602bb1069843, recipe v2
    .word 0xe4b2e904
    .word 0x5d36b0ef
