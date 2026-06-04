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

# -----------------------------------------------------------------
# Shared semihosting helpers (local, not exported)
# -----------------------------------------------------------------

# __lang_writec ( a0:byte -- )
# Emit low byte of a0 via RISC-V semihosting SYS_WRITEC.
# Preserves s2-s11 (callee-saved).
__lang_writec:
    sw a0, -4(sp)
    addi sp, sp, -4
    li a0, 0x03                     # SYS_WRITEC
    mv a1, sp
    semihost_call
    addi sp, sp, 4
    ret

# __lang_sys_exit ( a1:reason -- )
# Terminate via RISC-V semihosting SYS_EXIT with reason code in a1.
# Never returns.
__lang_sys_exit:
    sw a1, -4(sp)
    addi sp, sp, -4
    mv a1, sp
    li a0, 0x18                     # SYS_EXIT
    semihost_call
    addi sp, sp, 4
    ebreak

# __lang_fail_exit ( -- )
# Terminate with ADP_Stopped_ApplicationExit (0x20026).
# Never returns.
__lang_fail_exit:
    li a1, 0x20026
    j __lang_sys_exit

# -----------------------------------------------------------------
# Entry point
# -----------------------------------------------------------------

.globl __lang_start
.type __lang_start, @function
__lang_start:
    # DS pointer s2 = __lang_ds_base (low address, grows upward)
    la s2, __lang_ds_base
    # DS limit s3 = __lang_ds_limit (exclusive upper bound)
    la s3, __lang_ds_limit
    # Initialize high-water to DS base
    la a0, __lang_ds_high
    sw s2, 0(a0)

    jal w_1f5962a2ce9803c8          # call main ( -- i64 )

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
# Trap handlers
# -----------------------------------------------------------------

.globl __lang_trap
.type __lang_trap, %function
.globl __lang_trap_loc
.type __lang_trap_loc, %function
.globl __stack_overflow
.type __stack_overflow, %function
__lang_trap:
__lang_trap_loc:
__stack_overflow:
    j __lang_fail_exit

# -----------------------------------------------------------------
# testio words — RISC-V semihosting via shared snippet
# -----------------------------------------------------------------
.include "../include/semihosting-riscv.s"

# -----------------------------------------------------------------
# Module modpack section (S2 Phase 14)
# -----------------------------------------------------------------
.section .modpack
.globl __lang_modpack_start
__lang_modpack_start:
.globl __lang_modpack_end
__lang_modpack_end:

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

.globl __lang_expected_abi_hash
__lang_expected_abi_hash:
    .word 0x53048547
    .word 0x0445187d

    # Native stack — 32 KB (grows downward, sp initialized by crt0)
    .space 32768
__stack_top:
