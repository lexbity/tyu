# RISC-V RV32 bare-metal runtime (QEMU virt machine)
#
# DRAM at 0x80000000; code and read-only data in DRAM;
# BSS (DS, high-water, native stack) follows.
#
# ABI contract (abi-contract 4.4.1 / 4.4.2):
#   DS pointer = s2 (x18, upward-growing: push = addi s2, +N, pop = addi s2, -N)
#   slot_bytes = 4
#   DS limit   = s3 (x19)

.macro semihost_call
    slli x0, x0, 0x1f
    ebreak
    srai x0, x0, 0x7
.endm

.section .text
.extern __lang_entry

# -----------------------------------------------------------------
# Shared semihosting helpers (local, not exported)
# -----------------------------------------------------------------

# __lang_writec ( a0:byte -- )
# Emit low byte of a0 via RISC-V semihosting SYS_WRITEC.
# Preserves s2-s11 (callee-saved) AND a0, so callers emitting a run of equal
# bytes (e.g. `li a0, 0; jal __lang_writec` repeated) keep their value across
# calls instead of seeing the clobbered SYS_WRITEC operation number.
__lang_writec:
    sw a0, -4(sp)
    addi sp, sp, -4
    li a0, 0x03                     # SYS_WRITEC
    mv a1, sp
    semihost_call
    lw a0, 0(sp)                    # restore caller's byte value
    addi sp, sp, 4
    ret

# __lang_sys_exit ( a1:reason -- )
# Terminate via RISC-V semihosting SYS_EXIT with reason code in a1.
# Never returns.
# For 32-bit RISC-V semihosting, SYS_EXIT takes the reason code DIRECTLY in the
# parameter register (a1) — not a pointer to a block (that is the 64-bit form).
# Passing a pointer makes QEMU see an unrecognized reason and exit with status 1
# instead of terminating cleanly on ADP_Stopped_ApplicationExit.
__lang_sys_exit:
    li a0, 0x18                     # SYS_EXIT; a1 already holds the reason
    semihost_call
    ebreak                          # should not reach here

# __lang_fail_exit ( -- )
# Terminate with ADP_Stopped_ApplicationExit (0x20026).
# Never returns.
__lang_fail_exit:
    li a1, 0x20026
    j __lang_sys_exit

# -----------------------------------------------------------------
# Entry point
# -----------------------------------------------------------------
#
# Placed in `.text.init` so the linker can position it at the very start of
# DRAM (0x80000000).  With `-bios none`, the `virt` machine resets straight to
# 0x80000000, so the first instruction there MUST be the entry point.
.section .text.init

.globl __lang_start
.type __lang_start, @function
__lang_start:
    # Native call/scratch stack sp = __stack_top (grows downward).  Booting
    # with `-bios none` means no firmware has set sp, so we must establish it
    # before any helper that spills to the native stack (e.g. __lang_writec).
    la sp, __stack_top
    # DS pointer s2 = __lang_ds_base (low address, grows upward)
    la s2, __lang_ds_base
    # DS limit s3 = __lang_ds_limit (exclusive upper bound)
    la s3, __lang_ds_limit
    # Initialize high-water to DS base
    la a0, __lang_ds_high
    sw s2, 0(a0)
    # Initialize V-once flag
    la a0, __lang_v_emitted
    sw zero, 0(a0)

    jal __lang_entry                # static: call main; dynamic: load lmod then run main

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
# testio words — RISC-V semihosting via shared snippet
# -----------------------------------------------------------------
.include "../include/semihosting-riscv.s"

# -----------------------------------------------------------------
# platform.gpio words — synthetic latch for smoke tests
# -----------------------------------------------------------------
.globl w_a6b1202e57aa7cc9
.type w_a6b1202e57aa7cc9, @function
w_a6b1202e57aa7cc9:
    addi s2, s2, -8
    la t0, __lang_gpio_state
    sb zero, 0(t0)
    ret

.globl w_eb1d0a3c5e7c2e92
.type w_eb1d0a3c5e7c2e92, @function
w_eb1d0a3c5e7c2e92:
    addi s2, s2, -8
    lbu t1, 4(s2)
    la t0, __lang_gpio_state
    sb t1, 0(t0)
    ret

.globl w_034a1ff17acf93d3
.type w_034a1ff17acf93d3, @function
w_034a1ff17acf93d3:
    addi s2, s2, -4
    la t0, __lang_gpio_state
    lbu t1, 0(t0)
    sw t1, 0(s2)
    ret

# -----------------------------------------------------------------
# platform.uart/time words — semihosting + monotonic stub
# -----------------------------------------------------------------
.globl w_6f29c37992fecaf8
.type w_6f29c37992fecaf8, @function
w_6f29c37992fecaf8:
    addi s2, s2, -4
    ret

.globl w_38276faeb09bf91e
.type w_38276faeb09bf91e, @function
w_38276faeb09bf91e:
    addi s2, s2, -4
    lw a0, 0(s2)
    jal __lang_writec
    ret

.globl w_38126baeb0899648
.type w_38126baeb0899648, @function
w_38126baeb0899648:
    addi s2, s2, -4
    sw zero, 0(s2)
    addi s2, s2, 4
    sw zero, 0(s2)
    addi s2, s2, 4
    ret

.globl w_6a5791a972f2fbd0
.type w_6a5791a972f2fbd0, @function
w_6a5791a972f2fbd0:
    la t0, __lang_time_counter
    lw t1, 0(t0)
    lw t2, 4(t0)
    addi t1, t1, 1
    bnez t1, 1f
    addi t2, t2, 1
1:
    sw t1, 0(t0)
    sw t2, 4(t0)
    sw t1, 0(s2)
    addi s2, s2, 4
    sw t2, 0(s2)
    addi s2, s2, 4
    ret

.globl w_46f6f74f7859ca64
.type w_46f6f74f7859ca64, @function
w_46f6f74f7859ca64:
    j __lang_fail_exit

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
.section .bss

    # Data stack — 16 KB. s2 starts at __lang_ds_base (low address)
    # and grows upward. s3 = __lang_ds_limit (upper bound, exclusive).
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

.globl __lang_gpio_state
__lang_gpio_state:
    .word 0

.globl __lang_time_counter
__lang_time_counter:
    .word 0
    .word 0

.globl __mmio_mem
__mmio_mem:
    .space 4096

    # Region allocator arena (P7): 4096-byte bump arena + 16 slot records.
    .balign 8
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

.section .data
.globl __lang_expected_abi_hash
__lang_expected_abi_hash:
    # compute_abi_hash(ARCH_TAG_RISCV=3, slot=4, word=32, MODINFO_VER=4) = 0x49d5b84f8a5a3c42, recipe v2
    .word 0xe430bd85
    .word 0xf6dd34a3

    # return to BSS for the native stack
    .section .bss
    # Guard zone below the usable stack: when a word prologue detects
    # sp < __lang_stack_limit it branches to __stack_overflow, which then runs
    # (emits its diagnostic) using this reserved headroom.
    .space 1024
    .globl __lang_stack_limit
__lang_stack_limit:
    # Native stack — 32 KB (grows downward, sp initialized by crt0)
    .space 32768
__stack_top:
