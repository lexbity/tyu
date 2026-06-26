@ RP2350 ARMv7-M runtime, forked for the custom board pack.
@
@ The generic ARM runtime stays in runtime/armv7m-unknown-none/.
@ This copy binds the portable surface to RP2350 hardware:
@   - GPIO via IO_BANK0 + PADS_BANK0 + SIO
@   - UART via UART0
@   - time via TIMER0

.syntax unified
.thumb

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
.equ IO_CTRL_FUNCSEL,  0x1F
.equ FUNCSEL_UART0,   0x02
.equ FUNCSEL_SIO,     0x05

.equ PAD_IE,          0x040
.equ PAD_ISO,         0x100

.section .vectors, "a", %progbits
.type _vectors, %object
_vectors:
    .word __stack_top
    .word __lang_start + 1
    .word __lang_trap + 1
    .word __lang_trap + 1
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
    tst r2, #0x20                  @ TXFF
    bne 1b
    str r0, [r1, #0x00]
    bx lr

.type __lang_uart_init, %function
__lang_uart_init:
    push {r1, r2, r3, lr}

    @ Configure GPIO0/1 for UART0.
    ldr r1, =PADS_BANK0_BASE
    ldr r2, [r1, #0x00]
    orr r2, r2, #PAD_IE
    bic r2, r2, #PAD_ISO
    str r2, [r1, #0x00]
    ldr r2, [r1, #0x04]
    orr r2, r2, #PAD_IE
    bic r2, r2, #PAD_ISO
    str r2, [r1, #0x04]

    ldr r1, =IO_BANK0_BASE
    ldr r2, =FUNCSEL_UART0
    str r2, [r1, #IO_CTRL_OFF]
    str r2, [r1, #0x0c]

    @ Fixed 115200 baud from the 125 MHz peri clock.
    ldr r1, =UART0_BASE
    movs r2, #0
    str r2, [r1, #0x30]            @ disable
    ldr r2, =67
    str r2, [r1, #0x24]
    ldr r2, =52
    str r2, [r1, #0x28]
    ldr r2, =0x70                  @ 8N1, FIFO enabled
    str r2, [r1, #0x2C]
    ldr r2, =0x301                 @ UARTEN | TXE | RXE
    str r2, [r1, #0x30]

    pop {r1, r2, r3, lr}
    bx lr

.type __lang_fail_exit, %function
__lang_fail_exit:
    b .

.global __lang_start
.type __lang_start, %function
__lang_start:
    ldr r4, =__lang_ds_base
    ldr r5, =__lang_ds_limit
    ldr r0, =__lang_ds_high
    str r4, [r0]
    ldr r0, =__lang_v_emitted
    movs r1, #0
    str r1, [r0]
    bl __lang_uart_init
    bl w_1f5962a2ce9803c8
    b __lang_trap

.global __lang_trap
.type __lang_trap, %function
__lang_trap:
    push {r4, r5, r6, r7, r8, r9, r10, r11, lr}
    mov r6, r4
    ldr r1, =__lang_ds_base
    sub r6, r6, r1
    lsr r6, r6, #2
    mov r9, r0
    mov r10, #0
    mov r11, #0
    mov r7, #0
    mov r8, #0
    bl emit_diag

.global __lang_hardfault
.type __lang_hardfault, %function
__lang_hardfault:
    movs r0, #0
    b __lang_trap

@ -----------------------------------------------------------------
@ __stack_overflow — data-stack overflow detected at runtime
@
@ Entered from the emitted DS bounds check (sp < __lang_stack_limit).
@ No payload registers set — valid=0, trap_code=10.  Uses the same
@ emit_diag register convention as __lang_trap.
@ -----------------------------------------------------------------
.global __stack_overflow
.type __stack_overflow, %function
__stack_overflow:
    push {r4, r5, r6, r7, r8, r9, r10, r11, lr}
    mov r6, r4
    ldr r1, =__lang_ds_base
    sub r6, r6, r1
    lsr r6, r6, #2
    mov r9, #10                     @ trap_code = STACK_OVERFLOW
    mov r10, #0                     @ valid = 0
    mov r11, #0                     @ source_line = 0
    mov r7, #0                      @ word_hash low = 0
    mov r8, #0                      @ word_hash high = 0
    bl emit_diag
    @ never returns

.type emit_diag, %function
emit_diag:
    ldr r0, =__lang_v_emitted
    ldr r0, [r0]
    cmp r0, #0
    bne emit_diag_header
    mov r0, #0x56
    bl __lang_writec
    mov r0, #1
    bl __lang_writec
    movs r0, #0
    bl __lang_writec
    mov r0, #1
    bl __lang_writec
    ldr r0, =__lang_v_emitted
    movs r1, #1
    str r1, [r0]

emit_diag_header:
    mov r0, #0x44
    bl __lang_writec
    mov r0, #35
    bl __lang_writec
    movs r0, #0
    bl __lang_writec
    mov r0, #1
    bl __lang_writec
    mov r0, #1
    bl __lang_writec
    mov r0, r10
    bl __lang_writec
    mov r0, r9
    bl __lang_writec
    lsrs r0, r9, #8
    bl __lang_writec
    mov r0, r11
    bl __lang_writec
    lsrs r0, r11, #8
    bl __lang_writec
    lsrs r0, r11, #16
    bl __lang_writec
    lsrs r0, r11, #24
    bl __lang_writec
    mov r0, r7
    bl __lang_writec
    lsrs r0, r7, #8
    bl __lang_writec
    lsrs r0, r7, #16
    bl __lang_writec
    lsrs r0, r7, #24
    bl __lang_writec
    mov r0, r8
    bl __lang_writec
    lsrs r0, r8, #8
    bl __lang_writec
    lsrs r0, r8, #16
    bl __lang_writec
    lsrs r0, r8, #24
    bl __lang_writec
    movs r0, #0
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    mov r0, r6
    bl __lang_writec
    lsrs r0, r6, #8
    bl __lang_writec
    lsrs r0, r6, #16
    bl __lang_writec
    lsrs r0, r6, #24
    bl __lang_writec
    movs r0, #0xFF
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    bl __lang_writec
    movs r0, #0
    bl __lang_writec
    bl __lang_writec
    b __lang_fail_exit

@ -----------------------------------------------------------------
@ testio words — UART-backed RP2350 implementation
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
    @ r0 = pin, r1 = mode
    ldr r2, =PADS_BANK0_BASE
    add r3, r2, r0, lsl #2
    ldr r2, [r3]
    orr r2, r2, #PAD_IE
    bic r2, r2, #PAD_ISO
    str r2, [r3]
    ldr r2, =IO_BANK0_BASE
    add r3, r2, r0, lsl #3
    add r3, r3, #IO_CTRL_OFF
    ldr r2, =FUNCSEL_SIO
    str r2, [r3]
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
    tst r0, #0x10                  @ RXFE
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
    ldr r0, =TIMER0_BASE
1:  ldr r1, [r0, #0x08]
    ldr r2, [r0, #0x0c]
    ldr r3, [r0, #0x08]
    cmp r1, r3
    bne 1b
    str r2, [r4]
    adds r4, r4, #4
    str r1, [r4]
    adds r4, r4, #4
    bx lr

.global w_46f6f74f7859ca64
.type w_46f6f74f7859ca64, %function
w_46f6f74f7859ca64:
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

@ Guard zone below the usable native stack: when a word prologue detects
@ sp < __lang_stack_limit it branches to __stack_overflow, which then runs
@ (emits its diagnostic) using this reserved headroom.
    .space 1024
.global __lang_stack_limit
__lang_stack_limit:
    .space 32768
.global __stack_top
__stack_top:

.section .data, "aw"
.global __lang_expected_abi_hash
__lang_expected_abi_hash:
    .word 0xe4b2e904
    .word 0x5d36b0ef
