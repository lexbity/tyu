# RP2350 RISC-V (Hazard3) metal runtime — board pack fork (P8).
#
# The generic RV32 runtime stays in runtime/riscv32-unknown-none/. This copy
# binds the portable surface to RP2350 hardware and brings the board up from
# the bootrom handoff state (same sequence as the ARM fork):
#   - XOSC (12 MHz crystal), PLL_SYS -> clk_sys = clk_peri = 150 MHz
#   - clk_ref = XOSC, TIMER0 tick generator = 1 us            (DS2 8.5)
#   - resets released for every peripheral this runtime drives
#   - GPIO via IO_BANK0 + PADS_BANK0 + SIO; UART via UART0; time via TIMER0
#   - platform.mem.region words: reference bump allocator (P7, metal.trust)
#
# Every device constant is transcribed from RP-008373-DS-2; register offsets
# and bit positions are identical across the two ISAs (one register block).
# See platforms/rp2350/docs/descriptor-notes.md.

.equ XOSC_BASE,       0x40048000   # DS2 Table 13 (APB address map)
.equ CLOCKS_BASE,     0x40010000
.equ PLL_SYS_BASE,    0x40050000
.equ RESETS_BASE,     0x40020000
.equ IO_BANK0_BASE,   0x40028000
.equ PADS_BANK0_BASE, 0x40038000
.equ UART0_BASE,      0x40070000
.equ TIMER0_BASE,     0x400b0000
.equ SIO_BASE,        0xd0000000
.equ TICKS_BASE,      0x40108000

# Hazard3 machine trap vector (mtvec, direct mode). Reset leaves the bootrom's
# mtvec in place; route exceptions into this runtime's trap dump instead.
.equ H3_MTVEC,        0x305

# XOSC (DS2 8.2): FREQ_RANGE 1-15 MHz code 0xaa0 in [11:0], ENABLE key 0xfff
# in [23:12]; STATUS.STABLE is bit 31; STARTUP.DELAY in [13:0].
.equ XOSC_CTRL_VAL,    0x0fff0aa0
.equ XOSC_STARTUP_VAL, 64         # 64 * 256 xtal cycles (~1.4 ms @ 12 MHz)

# RESETS bit positions (DS2 7.5.2): UART0=26, TIMER0=23, PLL_SYS=14,
# PADS_BANK0=9, IO_BANK0=6.  RESET at +0x00, RESET_DONE at +0x08.
.equ RESETS_MASK,     0x04804240  # (1<<26)|(1<<23)|(1<<14)|(1<<9)|(1<<6)

# CLOCKS offsets (DS2 8.1.7) — see the ARM fork for the field layout.
.equ CLK_REF_CTRL_OFF,     0x30
.equ CLK_REF_SELECTED_OFF, 0x38
.equ CLK_SYS_CTRL_OFF,     0x3c
.equ CLK_SYS_SELECTED_OFF, 0x44
.equ CLK_PERI_CTRL_OFF,    0x48
.equ CLK_SYS_CTRL_AUX_PLL, 0x1    # SRC=1 (aux), AUXSRC=0 (pll_sys)
.equ CLK_PERI_CTRL_ENABLE, 0x800  # ENABLE, AUXSRC=0 (clk_sys)
.equ CLK_REF_CTRL_XOSC,    0x2    # SRC=2 (xosc_clksrc)

# PLL_SYS (DS2 8.6): 12 MHz / 1 * 125 = 1500 MHz VCO, / 5 / 2 = 150 MHz.
.equ PLL_CS_REFDIV_1,   0x1
.equ PLL_PWR_ALL_OFF,   0x2d      # PD | DSMPD | POSTDIVPD | VCOPD
.equ PLL_PWR_VCO_ONLY,  0x20      # VCOPD
.equ PLL_FBDIV_125,     125
.equ PLL_PRIM_DIV5_2,   0x52000   # POSTDIV1=5 | POSTDIV2=2 << 12

# Tick generators run from clk_ref (DS2 8.5): 12 cycles of the 12 MHz XOSC
# give the 1 us tick TIMER0 defaults to.
.equ TICKS_TIMER0_CTRL_OFF,   0x18
.equ TICKS_TIMER0_CYCLES_OFF, 0x1c

# IO_BANK0 (DS2 9.11): each pin is { STATUS +8n, CTRL +8n+4 }.
.equ IO_CTRL_OFF,     0x004       # GPIO0_CTRL (UART0 TX function select)
.equ FUNCSEL_UART0,   0x02
.equ FUNCSEL_SIO,     0x05

# SIO (DS2 3.1.11): RP2350 interleaves the GPIO_HI bank.
.equ SIO_GPIO_IN,      0x004
.equ SIO_GPIO_OUT_SET, 0x018
.equ SIO_GPIO_OUT_CLR, 0x020
.equ SIO_GPIO_OE_SET,  0x038
.equ SIO_GPIO_OE_CLR,  0x040

# PADS_BANK0 (DS2 9.11.3): IE bit 6, ISO bit 8.
.equ PAD_IE,          0x040
.equ PAD_ISO,         0x100

# UART0 at 115200 baud from clk_peri = 150 MHz (DS2 12.1).
.equ UART_IBRD_150MHZ_115200, 81
.equ UART_FBRD_150MHZ_115200, 24

.section .text

.globl __lang_writec
__lang_writec:
    j __lang_uart_putc

__lang_uart_putc:
    li t0, UART0_BASE
1:  lw t1, 0x18(t0)
    andi t1, t1, 0x20              # FR.TXFF
    bnez t1, 1b
    sw a0, 0(t0)
    ret

# ---------------------------------------------------------------------------
# Board bring-up (runs from the bootrom handoff clock state).
# Preserves s2/s3 (DS pointer / limit).
# ---------------------------------------------------------------------------
__lang_clocks_init:
    addi sp, sp, -4
    sw ra, 0(sp)

    # XOSC on and stable (DS2 8.2)
    li t0, XOSC_BASE
    li t1, XOSC_CTRL_VAL
    sw t1, 0(t0)                   # CTRL
    li t1, XOSC_STARTUP_VAL
    sw t1, 0x0c(t0)                # STARTUP
1:  lw t1, 0x04(t0)                # STATUS
    bltz t1, 2f                    # bit 31 (STABLE) set -> negative as signed
    j 1b
2:

    # Release resets for UART0, TIMER0, PLL_SYS, PADS_BANK0, IO_BANK0
    li t0, RESETS_BASE
    lw t1, 0(t0)                   # RESET
    li t2, RESETS_MASK
    not t2, t2
    and t1, t1, t2
    sw t1, 0(t0)
3:  lw t1, 0x08(t0)                # RESET_DONE
    and t1, t1, t2
    bne t1, t2, 3b

    # PLL_SYS -> 150 MHz (DS2 8.6)
    li t0, PLL_SYS_BASE
    li t1, PLL_CS_REFDIV_1
    sw t1, 0(t0)                   # CS: REFDIV = 1
    li t1, PLL_PWR_ALL_OFF
    sw t1, 0x04(t0)                # PWR: everything off
    li t1, PLL_FBDIV_125
    sw t1, 0x08(t0)                # FBDIV_INT = 125
    li t1, PLL_PWR_VCO_ONLY
    sw t1, 0x04(t0)                # PWR: VCO only
4:  lw t1, 0(t0)                   # CS
    bltz t1, 5f                    # bit 31 (LOCK) set
    j 4b
5:  li t1, PLL_PRIM_DIV5_2
    sw t1, 0x0c(t0)                # PRIM: /5, /2
    sw zero, 0x04(t0)              # PWR: outputs on

    # clk_sys -> aux(PLL_SYS); poll the glitchless mux until settled
    li t0, CLOCKS_BASE
    li t1, CLK_SYS_CTRL_AUX_PLL
    sw t1, CLK_SYS_CTRL_OFF(t0)
6:  lw t1, CLK_SYS_SELECTED_OFF(t0)
    li t2, 2                       # one-hot bit 1 = aux position
    bne t1, t2, 6b

    # clk_peri <- clk_sys (drives UART0)
    li t1, CLK_PERI_CTRL_ENABLE
    sw t1, CLK_PERI_CTRL_OFF(t0)

    # clk_ref -> XOSC (drives the tick generators)
    li t1, CLK_REF_CTRL_XOSC
    sw t1, CLK_REF_CTRL_OFF(t0)
7:  lw t1, CLK_REF_SELECTED_OFF(t0)
    li t2, 4                       # one-hot bit 2 = xosc position
    bne t1, t2, 7b

    # TIMER0 tick: 12 cycles of the 12 MHz XOSC = 1 us
    li t0, TICKS_BASE
    sw zero, TICKS_TIMER0_CTRL_OFF(t0)
    li t1, 12
    sw t1, TICKS_TIMER0_CYCLES_OFF(t0)
    li t1, 1
    sw t1, TICKS_TIMER0_CTRL_OFF(t0)
8:  lw t1, TICKS_TIMER0_CTRL_OFF(t0)
    andi t1, t1, 2                 # RUNNING
    beqz t1, 8b

    lw ra, 0(sp)
    addi sp, sp, 4
    ret

__lang_uart_init:
    addi sp, sp, -4
    sw ra, 0(sp)

    # GPIO0/1 pads: input enable on, release isolation (DS2 9.11.3)
    li t0, PADS_BANK0_BASE
    lw t1, 0x00(t0)                # GPIO0 pad
    ori t1, t1, PAD_IE
    li t2, ~PAD_ISO
    and t1, t1, t2
    sw t1, 0x00(t0)
    lw t1, 0x04(t0)                # GPIO1 pad
    ori t1, t1, PAD_IE
    and t1, t1, t2
    sw t1, 0x04(t0)

    # GPIO0/1 -> UART0 function (CTRL at +8n+4, DS2 9.11)
    li t0, IO_BANK0_BASE
    li t1, FUNCSEL_UART0
    sw t1, IO_CTRL_OFF(t0)         # GPIO0_CTRL
    sw t1, 0x0c(t0)                # GPIO1_CTRL

    # 115200 8N1, FIFOs on, from clk_peri = 150 MHz
    li t0, UART0_BASE
    sw zero, 0x30(t0)              # CR: disable while configuring
    li t1, 0x7ff
    sw t1, 0x44(t0)                # ICR: clear pending interrupts
    li t1, UART_IBRD_150MHZ_115200
    sw t1, 0x24(t0)                # IBRD
    li t1, UART_FBRD_150MHZ_115200
    sw t1, 0x28(t0)                # FBRD
    li t1, 0x70                    # LCR_H: 8N1, FIFO enable
    sw t1, 0x2c(t0)
    li t1, 0x301                   # CR: UARTEN | TXE | RXE
    sw t1, 0x30(t0)

    lw ra, 0(sp)
    addi sp, sp, 4
    ret

__lang_fail_exit:
    j .

.globl __lang_start
__lang_start:
    # data stack (DS) init — s2/s3 are the compiled-code DS convention
    la s2, __lang_ds_base
    la s3, __lang_ds_limit
    la t0, __lang_ds_high
    sw s2, 0(t0)

    # route machine traps into this runtime's dump (mtvec, direct mode).
    # The toolchain assembles with -march=rv32im, which does not imply the
    # zicsr extension, so enable it locally for the CSR write.
    la t0, __lang_trap
    .option push
    .option arch, +zicsr
    csrw H3_MTVEC, t0
    .option pop

    # zero .bss: on real silicon SRAM content is not guaranteed across
    # reset, and the DS high-water marks and the region allocator rely on
    # zero-initialized statics.
    la t0, __bss_start
    la t1, __bss_end
0:
    bgeu t0, t1, 9f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 0b
9:

    call __lang_clocks_init
    call __lang_uart_init
    call __lang_entry              # static: call main; dynamic: load lmod then run main


.globl __lang_after_main
.type __lang_after_main, @function
__lang_after_main:

    # Pop exit code from DS (i64 = two 4-byte slots)
    addi s2, s2, -8
    lw a0, 0(s2)                    # a0 = low 32 bits of exit code (discarded)

    # Emit high-water: 'H' + u32-le (peak DS depth in slots)
    la a1, __lang_ds_high
    lw a0, 0(a1)
    la a1, __lang_ds_base
    sub a0, a0, a1
    srli a0, a0, 2
    mv t0, a0                       # save slot count in t0

    li a0, 0x48                     # 'H'
    jal __lang_writec
    mv a0, t0                       # byte 0 (LSB)
    jal __lang_writec
    srli a0, t0, 8                  # byte 1
    jal __lang_writec
    srli a0, t0, 16                 # byte 2
    jal __lang_writec
    srli a0, t0, 24                 # byte 3 (MSB)
    jal __lang_writec

    j __lang_fail_exit

# -----------------------------------------------------------------
# Diagnostic D record emitter (shared)
#
# Emits V (once) + framed D header via __lang_writec, then
# falls through to __lang_fail_exit.
#
# Input registers (preserved by __lang_writec, s2-s11 callee-saved):
#   s4 = ds_depth (u32, in slots)
#   s5 = trap_code (u32, low 16 bits used)
#   s6 = valid (0 or 1)
#   s7 = source_line (u32)
#   s8 = word_hash low 32 bits
#   s9 = word_hash high 32 bits
# -----------------------------------------------------------------
emit_diag:
    # Emit V version record at most once.
    la t0, __lang_v_emitted
    lw t0, 0(t0)
    bnez t0, emit_diag_header
    li a0, 0x56                  # 'V'
    jal __lang_writec
    li a0, 1                     # len = 1 (u16-le)
    jal __lang_writec
    li a0, 0                     # len high byte
    jal __lang_writec
    li a0, 1                     # version = 1
    jal __lang_writec
    la t0, __lang_v_emitted
    li t1, 1
    sw t1, 0(t0)

emit_diag_header:
    # D marker
    li a0, 0x44
    jal __lang_writec

    # Total payload length = 35 (u16-le, header-only, slot_count=0)
    li a0, 35
    jal __lang_writec
    li a0, 0
    jal __lang_writec

    # Byte 0: DiagRecord.version = 1
    li a0, 1
    jal __lang_writec

    # Byte 1: origin = IN_GUEST (1)
    li a0, 1
    jal __lang_writec

    # Byte 2: valid = s6
    mv a0, s6
    jal __lang_writec

    # Bytes 3-4: trap_code (u16-le) from s5
    mv a0, s5
    jal __lang_writec
    srli a0, s5, 8
    jal __lang_writec

    # Bytes 5-8: source_line (u32-le) from s7
    mv a0, s7
    jal __lang_writec
    srli a0, s7, 8
    jal __lang_writec
    srli a0, s7, 16
    jal __lang_writec
    srli a0, s7, 24
    jal __lang_writec

    # Bytes 9-16: word_hash (u64-le) from s8 (low) : s9 (high)
    mv a0, s8
    jal __lang_writec
    srli a0, s8, 8
    jal __lang_writec
    srli a0, s8, 16
    jal __lang_writec
    srli a0, s8, 24
    jal __lang_writec
    mv a0, s9
    jal __lang_writec
    srli a0, s9, 8
    jal __lang_writec
    srli a0, s9, 16
    jal __lang_writec
    srli a0, s9, 24
    jal __lang_writec

    # Bytes 17-24: trap_pc (u64-le) = 0 (not available on header-only path)
    li a0, 0
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec

    # Bytes 25-28: ds_depth (u32-le) from s4
    mv a0, s4
    jal __lang_writec
    srli a0, s4, 8
    jal __lang_writec
    srli a0, s4, 16
    jal __lang_writec
    srli a0, s4, 24
    jal __lang_writec

    # Bytes 29-32: ds_declared = 0xFFFFFFFF (unknown / T)
    li a0, 0xFF
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec

    # Bytes 33-34: slot_count = 0 (header-only)
    li a0, 0
    jal __lang_writec
    jal __lang_writec

    j __lang_fail_exit

# Remaining runtime code returns to the ordinary `.text` section.
.section .text

# -----------------------------------------------------------------
# __lang_trap_loc — trap with source location (debug_trap_loc=true)
#
# Register contract:
#   a0 = trap_code
#   a1 = valid (1)
#   a2 = source_line
#   a3 = word_hash low 32 bits
#   a4 = word_hash high 32 bits
# -----------------------------------------------------------------
.globl __lang_trap_loc
.type __lang_trap_loc, @function
__lang_trap_loc:
    # Compute ds_depth from s2 (DS pointer, preserved).
    mv s4, s2
    la t0, __lang_ds_base
    sub s4, s4, t0
    srli s4, s4, 2                   # s4 = ds_depth (slots)

    # Save trap payload to callee-saved registers.
    mv s5, a0                         # trap_code
    mv s6, a1                         # valid
    mv s7, a2                         # source_line
    mv s8, a3                         # word_hash low
    mv s9, a4                         # word_hash high

    j emit_diag

# -----------------------------------------------------------------
# __lang_trap — generic trap from compiled code
#
# Entered via:  j __lang_trap
# Register contract:
#   a0 = trap_code (set by compiler), a1/a2/a3/a4 = undefined
# -----------------------------------------------------------------
.globl __lang_trap
.type __lang_trap, %function
__lang_trap:
    mv s4, s2
    la t0, __lang_ds_base
    sub s4, s4, t0
    srli s4, s4, 2

    mv s5, a0                         # trap_code
    li s6, 0                           # valid = 0
    li s7, 0                           # source_line = 0
    li s8, 0                           # word_hash low = 0
    li s9, 0                           # word_hash high = 0

    j emit_diag

# -----------------------------------------------------------------
# __stack_overflow — data-stack overflow detected at runtime
# -----------------------------------------------------------------
.globl __stack_overflow
.type __stack_overflow, %function
__stack_overflow:
    mv s4, s2
    la t0, __lang_ds_base
    sub s4, s4, t0
    srli s4, s4, 2

    li s5, 10                          # trap_code = STACK_OVERFLOW
    li s6, 0                           # valid = 0
    li s7, 0                           # source_line = 0
    li s8, 0                           # word_hash low = 0
    li s9, 0                           # word_hash high = 0

    j emit_diag

# -----------------------------------------------------------------
# platform.mem.region words — reference bump allocator, ported from
# runtime/riscv32-unknown-none/runtime.asm (P7, D-6 metal.trust).
# -----------------------------------------------------------------
# platform.mem.region words — reference bump allocator (P7, D-6 metal.trust)
#
# Model matches the ARM/x86 hosted allocators: a static arena
# `__region_arena` (4096 bytes) in BSS with 16 slots of { base, size, off }.
# region-create carves a chunk from the arena and records it in a free slot;
# region-alloc bumps `off` within the slot; region-reset zeroes `off`;
# region-destroy frees the slot.
#
# DS convention (abi-contract): s2 = DS pointer, slot_bytes = 4, upward.
#   pop i64  = addi s2,s2,-8; lw rX,0(s2); lw rY,4(s2)
#   push i64 = sw rX,0(s2); addi s2,s2,4; sw rY,0(s2); addi s2,s2,4
# Trap: a0 = trap_code; j __lang_trap.
# Exhaustion raises REGION_EXHAUSTED (26).  Register discipline: only a0-a7 /
# t0-t6 (caller-saved) are used; s0-s11 preserved except s2 (DS pointer).
# -----------------------------------------------------------------

.globl w_7a5f795caa045668
.type w_7a5f795caa045668, @function
w_7a5f795caa045668:
    addi s2, s2, -8
    lw a0, 0(s2)                    # a0 = size (low)
    bnez a0, 1f
    li a0, 23                       # UNREACHABLE: zero-size region
    j __lang_trap
1:
    addi a0, a0, 7
    andi a0, a0, -8                 # align to 8
    li t0, 0                        # slot index
    la t1, __region_size
2:
    li t2, 16
    bge t0, t2, 3f                  # none free -> use __region_next
    slli t3, t0, 2
    add t3, t1, t3
    lw t4, 0(t3)
    beqz t4, 4f                     # free slot found
    addi t0, t0, 1
    j 2b
3:
    la t1, __region_next
    lw t0, 0(t1)
    li t2, 16
    bge t0, t2, 8f                  # all slots in use -> exhausted
    lw t3, 0(t1)
    addi t3, t3, 1
    sw t3, 0(t1)
4:
    # t0 = slot, a0 = aligned size
    la t1, __region_used
    lw t2, 0(t1)                    # used
    la t3, __region_arena
    add t3, t3, t2                  # base
    add t2, t2, a0                  # new used
    # end of this slot = base + size; must fit in arena
    add t4, t3, a0                  # end
    la t5, __region_arena_end
    bgt t4, t5, 8f                  # arena overrun -> exhausted
    la t1, __region_used
    sw t2, 0(t1)                    # used = new used
    la t1, __region_base
    slli t4, t0, 2
    add t4, t1, t4
    sw t3, 0(t4)                    # base[slot]
    la t1, __region_size
    slli t4, t0, 2
    add t4, t1, t4
    sw a0, 0(t4)                    # size[slot]
    la t1, __region_off
    slli t4, t0, 2
    add t4, t1, t4
    sw zero, 0(t4)                  # off[slot] = 0
    # return Region handle = slot index
    mv a0, t0
    li a1, 0
    sw a0, 0(s2)
    addi s2, s2, 4
    sw a1, 0(s2)
    addi s2, s2, 4
    ret
8:
    li a0, 26                       # REGION_EXHAUSTED
    j __lang_trap

.globl w_00433c33168e6701
.type w_00433c33168e6701, @function
w_00433c33168e6701:
    addi s2, s2, -8
    lw a1, 0(s2)                    # a1 = usize (size, low)
    addi s2, s2, -8
    lw a0, 0(s2)                    # a0 = Region handle
    li t0, 16
    bge a0, t0, 9f                  # bad handle -> UNREACHABLE
    la t1, __region_size
    slli t2, a0, 2
    add t2, t1, t2
    lw t3, 0(t2)                    # slot size
    beqz t3, 9f                     # dead slot -> UNREACHABLE
    addi a1, a1, 7
    andi a1, a1, -8                 # align request
    la t1, __region_off
    slli t2, a0, 2
    add t2, t1, t2
    lw t4, 0(t2)                    # off
    add t4, t4, a1                  # new off
    bgt t4, t3, 8f                  # region full -> REGION_EXHAUSTED
    sw t4, 0(t2)                    # off = new off
    la t1, __region_base
    slli t2, a0, 2
    add t2, t1, t2
    lw t3, 0(t2)                    # base
    la t1, __region_off
    slli t2, a0, 2
    add t2, t1, t2
    lw t4, 0(t2)                    # new off
    sub t4, t4, a1                  # old off
    add a0, t3, t4                  # ptr
    li a1, 0
    sw a0, 0(s2)
    addi s2, s2, 4
    sw a1, 0(s2)
    addi s2, s2, 4
    ret
8:
    li a0, 26                       # REGION_EXHAUSTED
    j __lang_trap
9:
    li a0, 23                       # UNREACHABLE
    j __lang_trap

.globl w_a52160bb1e22438b
.type w_a52160bb1e22438b, @function
w_a52160bb1e22438b:
    addi s2, s2, -8
    lw a0, 0(s2)
    li t0, 16
    bge a0, t0, 9f
    la t1, __region_size
    slli t2, a0, 2
    add t2, t1, t2
    lw t3, 0(t2)
    beqz t3, 9f
    la t1, __region_off
    slli t2, a0, 2
    add t2, t1, t2
    sw zero, 0(t2)
    ret
9:
    li a0, 23
    j __lang_trap

.globl w_3dc921382ce34c3e
.type w_3dc921382ce34c3e, @function
w_3dc921382ce34c3e:
    addi s2, s2, -8
    lw a0, 0(s2)
    li t0, 16
    bge a0, t0, 9f
    la t1, __region_size
    slli t2, a0, 2
    add t2, t1, t2
    lw t3, 0(t2)
    beqz t3, 9f
    la t1, __region_base
    slli t2, a0, 2
    add t2, t1, t2
    sw zero, 0(t2)
    la t1, __region_size
    slli t2, a0, 2
    add t2, t1, t2
    sw zero, 0(t2)
    la t1, __region_off
    slli t2, a0, 2
    add t2, t1, t2
    sw zero, 0(t2)
    ret
9:
    li a0, 23
    j __lang_trap

# -----------------------------------------------------------------
# BSS — DS region, high-water, native stack
# -----------------------------------------------------------------

.globl __region_arena
__region_arena:
    .space 4096
.globl __region_arena_end
__region_arena_end:
.globl __region_next
__region_next:
    .space 4
.globl __region_used
__region_used:
    .space 4
.globl __region_base
__region_base:
    .space 64
.globl __region_size
__region_size:
    .space 64
.globl __region_off
__region_off:
    .space 64


# -----------------------------------------------------------------
# testio words — UART-backed RP2350 implementation (metal.trust)
# -----------------------------------------------------------------
.globl w_accb676a903a06d9
.type w_accb676a903a06d9, @function
w_accb676a903a06d9:
    addi s2, s2, -8
    lw a0, 0(s2)
    jal __lang_writec
    ret

.globl w_eb06855547211672
.type w_eb06855547211672, @function
w_eb06855547211672:
    addi s2, s2, -4
    lw t0, 0(s2)
    lw t1, 0(t0)
    addi t0, t0, 8
    beqz t1, 1f
0:
    lbu a0, 0(t0)
    jal __lang_writec
    addi t0, t0, 1
    addi t1, t1, -1
    bnez t1, 0b
1:
    ret

.globl w_f91ca4f233247b4d
.type w_f91ca4f233247b4d, @function
w_f91ca4f233247b4d:
    addi s2, s2, -8
    lw a0, 0(s2)
    j __lang_fail_exit

.globl w_a6b1202e57aa7cc9
w_a6b1202e57aa7cc9:
    addi s2, s2, -8
    lw a0, 0(s2)
    lw a1, 4(s2)
    call __lang_gpio_init
    ret

.globl w_eb1d0a3c5e7c2e92
w_eb1d0a3c5e7c2e92:
    addi s2, s2, -8
    lw a0, 0(s2)
    lw a1, 4(s2)
    call __lang_gpio_write
    ret

.globl w_034a1ff17acf93d3
w_034a1ff17acf93d3:
    addi s2, s2, -4
    lw a0, 0(s2)
    call __lang_gpio_read
    sw a0, 0(s2)
    addi s2, s2, 4
    ret

.globl w_6f29c37992fecaf8
w_6f29c37992fecaf8:
    addi s2, s2, -4
    call __lang_uart_init
    ret

.globl w_38276faeb09bf91e
w_38276faeb09bf91e:
    addi s2, s2, -4
    lw a0, 0(s2)
    call __lang_uart_putc
    ret

.globl w_38126baeb0899648
w_38126baeb0899648:
    addi s2, s2, -4
    li t0, UART0_BASE
1:  lw t1, 0x18(t0)
    andi t1, t1, 0x10              # FR.RXFE
    bnez t1, 1b
    lw t1, 0(t0)
    sw t1, 0(s2)
    addi s2, s2, 4
    li t1, 1
    sw t1, 0(s2)
    addi s2, s2, 4
    ret

.globl w_6a5791a972f2fbd0
w_6a5791a972f2fbd0:
    # 64-bit microsecond time from TIMER0 (1 us tick configured at start).
    # TIMEHR only latches reliably via the TIMELR read, so read HR, LR, HR
    # and retry until the two high words agree (DS2 12.8).
    li t0, TIMER0_BASE
1:  lw t1, 0x08(t0)                # TIMEHR
    lw t2, 0x0c(t0)                # TIMELR (latches TIMEHR)
    lw t3, 0x08(t0)                # TIMEHR again
    bne t1, t3, 1b
    sw t2, 0(s2)                   # low word
    addi s2, s2, 4
    sw t3, 0(s2)                   # high word
    addi s2, s2, 4
    ret

.globl w_46f6f74f7859ca64
w_46f6f74f7859ca64:
    # platform.time.reboot: the RP2350 reboot path (REBOOT/RESCFG) is not
    # modeled by this runtime; park instead of silently returning.
    j .

__lang_gpio_init:
    li t0, PADS_BANK0_BASE
    slli t1, a0, 2
    add t1, t0, t1
    lw t2, 0(t1)
    ori t2, t2, PAD_IE
    li t3, ~PAD_ISO
    and t2, t2, t3
    sw t2, 0(t1)

    li t0, IO_BANK0_BASE
    slli t1, a0, 3
    add t1, t0, t1
    addi t1, t1, IO_CTRL_OFF
    li t2, FUNCSEL_SIO
    sw t2, 0(t1)

    # output enable via the SIO OE aliases (RP2350 offsets)
    li t0, 1
    sll t0, t0, a0
    beqz a1, 1f
    li t1, SIO_BASE
    sw t0, SIO_GPIO_OE_SET(t1)
    ret
1:  li t1, SIO_BASE
    sw t0, SIO_GPIO_OE_CLR(t1)
    ret

__lang_gpio_write:
    li t0, 1
    sll t0, t0, a0
    li t1, SIO_BASE
    beqz a1, 1f
    sw t0, SIO_GPIO_OUT_SET(t1)
    ret
1:  sw t0, SIO_GPIO_OUT_CLR(t1)
    ret

__lang_gpio_read:
    li t0, SIO_BASE
    li t1, 1
    sll t1, t1, a0
    lw t2, SIO_GPIO_IN(t0)
    and a0, t2, t1
    snez a0, a0
    ret

.section .bss
.globl __lang_ds_base
__lang_ds_base:
    .space 16384
.globl __lang_ds_limit
__lang_ds_limit:

.globl __lang_ds_high
__lang_ds_high:
    .word 0

.globl __lang_v_emitted
__lang_v_emitted:
    .word 0

# Guard zone below the overflow threshold (__lang_stack_limit): when a word
# prologue detects sp < __lang_stack_limit it branches to __stack_overflow,
# which runs using this reserved headroom.  The native stack itself lives at
# the top of SRAM (__stack_top, from the linker script).
    .space 1024
.globl __lang_stack_limit
__lang_stack_limit:

.section .data
.globl __lang_expected_abi_hash
__lang_expected_abi_hash:
    # compute_abi_hash(ARCH_TAG_RISCV=3, slot=4, word=32, MODINFO_VER=4) = 0x49d5b84f8a5a3c42, recipe v2
    .word 0xe430bd85
    .word 0xf6dd34a3
