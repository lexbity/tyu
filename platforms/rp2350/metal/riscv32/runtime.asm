# RP2350 RISC-V runtime, forked for the custom board pack.
#
# The generic RV32 runtime stays in runtime/riscv32-unknown-none/.
# This copy binds the portable surface to RP2350 hardware:
#   - GPIO via IO_BANK0 + PADS_BANK0 + SIO
#   - UART via UART0
#   - time via TIMER0

.equ SIO_BASE,        0xD0000000
.equ IO_BANK0_BASE,   0x40028000
.equ PADS_BANK0_BASE, 0x4001C000
.equ UART0_BASE,      0x40070000
.equ TIMER0_BASE,     0x400B0000

.equ SIO_GPIO_IN,     0x004
.equ SIO_GPIO_OUT_SET,0x014
.equ SIO_GPIO_OUT_CLR,0x018
.equ SIO_GPIO_OE_SET,  0x024
.equ SIO_GPIO_OE_CLR,  0x028

.equ IO_CTRL_OFF,     0x004
.equ FUNCSEL_UART0,   0x02
.equ FUNCSEL_SIO,     0x05

.equ PAD_IE,          0x040
.equ PAD_ISO,         0x100

.section .text

.globl __lang_writec
__lang_writec:
    j __lang_uart_putc

__lang_uart_putc:
    li t0, UART0_BASE
1:  lw t1, 0x18(t0)
    andi t1, t1, 0x20              # TXFF
    bnez t1, 1b
    sw a0, 0(t0)
    ret

__lang_gpio_read:
    li t0, SIO_BASE
    li t1, 1
    sll t1, t1, a0
    lw t2, 0x004(t0)
    and a0, t2, t1
    snez a0, a0
    ret

__lang_uart_init:
    # Fixed 115200 baud from the 125 MHz peri clock.
    li t0, PADS_BANK0_BASE
    lw t1, 0(t0)
    ori t1, t1, PAD_IE
    li t2, -257                    # ~PAD_ISO
    and t1, t1, t2
    sw t1, 0(t0)
    lw t1, 4(t0)
    ori t1, t1, PAD_IE
    and t1, t1, t2
    sw t1, 4(t0)

    li t0, IO_BANK0_BASE
    li t1, FUNCSEL_UART0
    sw t1, 0x004(t0)
    sw t1, 0x0c(t0)

    li t0, UART0_BASE
    sw zero, 0x30(t0)
    li t1, 67
    sw t1, 0x24(t0)
    li t1, 52
    sw t1, 0x28(t0)
    li t1, 0x70
    sw t1, 0x2c(t0)
    li t1, 0x301
    sw t1, 0x30(t0)
    ret

__lang_fail_exit:
    j .

.globl __lang_start
__lang_start:
    la s2, __lang_ds_base
    la s3, __lang_ds_limit
    la t0, __lang_ds_high
    sw s2, 0(t0)
    la t0, __lang_v_emitted
    sw zero, 0(t0)
    call __lang_uart_init
    call w_1f5962a2ce9803c8
    j __lang_trap

.globl __lang_trap
__lang_trap:
    mv s4, s2
    la t0, __lang_ds_base
    sub s4, s4, t0
    srli s4, s4, 2

    mv s5, a0
    li s6, 0
    li s7, 0
    li s8, 0
    li s9, 0

    j emit_diag

.globl __lang_hardfault
__lang_hardfault:
    li a0, 0
    j __lang_trap

# -----------------------------------------------------------------
# __stack_overflow — data-stack overflow detected at runtime
#
# Entered from the emitted DS bounds check (sp < __lang_stack_limit).
# No payload registers set — valid=0, trap_code=10.  Uses the same
# emit_diag register convention as __lang_trap.
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
    la t0, __lang_v_emitted
    lw t0, 0(t0)
    bnez t0, emit_diag_header
    li a0, 0x56
    jal __lang_writec
    li a0, 1
    jal __lang_writec
    li a0, 0
    jal __lang_writec
    li a0, 1
    jal __lang_writec
    la t0, __lang_v_emitted
    li t1, 1
    sw t1, 0(t0)

emit_diag_header:
    li a0, 0x44
    jal __lang_writec
    li a0, 35
    jal __lang_writec
    li a0, 0
    jal __lang_writec
    li a0, 1
    jal __lang_writec
    li a0, 1
    jal __lang_writec
    mv a0, s6
    jal __lang_writec
    mv a0, s5
    jal __lang_writec
    srli a0, s5, 8
    jal __lang_writec
    mv a0, s7
    jal __lang_writec
    srli a0, s7, 8
    jal __lang_writec
    srli a0, s7, 16
    jal __lang_writec
    srli a0, s7, 24
    jal __lang_writec
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
    li a0, 0
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    mv a0, s4
    jal __lang_writec
    srli a0, s4, 8
    jal __lang_writec
    srli a0, s4, 16
    jal __lang_writec
    srli a0, s4, 24
    jal __lang_writec
    li a0, 0xFF
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    jal __lang_writec
    li a0, 0
    jal __lang_writec
    jal __lang_writec
    j __lang_fail_exit

# -----------------------------------------------------------------
# testio words — UART-backed RP2350 implementation
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
    andi t1, t1, 0x10              # RXFE
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
    li t0, TIMER0_BASE
1:  lw t1, 0x08(t0)
    lw t2, 0x0c(t0)
    lw t3, 0x08(t0)
    bne t1, t3, 1b
    sw t2, 0(s2)
    addi s2, s2, 4
    sw t1, 0(s2)
    addi s2, s2, 4
    ret

.globl w_46f6f74f7859ca64
w_46f6f74f7859ca64:
    j .

__lang_gpio_init:
    li t0, PADS_BANK0_BASE
    slli t1, a0, 2
    add t1, t0, t1
    lw t2, 0(t1)
    ori t2, t2, PAD_IE
    li t3, -257                    # ~PAD_ISO
    and t2, t2, t3
    sw t2, 0(t1)

    li t0, IO_BANK0_BASE
    slli t1, a0, 3
    add t1, t0, t1
    addi t1, t1, IO_CTRL_OFF
    li t2, FUNCSEL_SIO
    sw t2, 0(t1)

    li t0, 1
    sll t0, t0, a0
    beqz a1, 1f
    li t1, SIO_BASE
    sw t0, 0x024(t1)
    ret
1:  li t1, SIO_BASE
    sw t0, 0x028(t1)
    ret

__lang_gpio_write:
    li t0, 1
    sll t0, t0, a0
    li t1, SIO_BASE
    beqz a1, 1f
    sw t0, 0x014(t1)
    ret
1:  sw t0, 0x018(t1)
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

# Guard zone below the usable native stack: when a word prologue detects
# sp < __lang_stack_limit it branches to __stack_overflow, which then runs
# (emits its diagnostic) using this reserved headroom.
    .space 1024
.globl __lang_stack_limit
__lang_stack_limit:
    .space 32768
.globl __stack_top
__stack_top:

.section .data
.globl __lang_expected_abi_hash
__lang_expected_abi_hash:
    .word 0xd11ba544
    .word 0xa7df1edb
