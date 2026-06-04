# RISC-V semihosting testio words
# Included by runtime/<triple>/runtime.asm
#
# Semihosting convention (RISC-V, same operations as ARM):
#   a0 = operation number
#   a1 = parameter block pointer
#   Call sequence:
#     slli x0, x0, 0x1f
#     ebreak
#     srai x0, x0, 0x7
#
# Operations:
#   SYS_WRITEC  = 0x03  write character (*a1)
#   SYS_EXIT    = 0x18  exit (a1 -> reason: u32)
#
# Data-stack discipline (abi-contract 4.4.2):
#   DS pointer register: TBD (Phase 11a)  -- uses s0 as placeholder
#   slot_bytes = 8 (riscv64, QEMU virt machine)
#   Upward-growing: push = addi s0, s0, 8; pop = addi s0, s0, -8

.macro semihost_call
    slli x0, x0, 0x1f
    ebreak
    srai x0, x0, 0x7
.endm

# -----------------------------------------------------------------
# testio.write-byte ( i64 -- )
# fnv1a_u64("testio.write-byte") = accb676a903a06d9
# -----------------------------------------------------------------
.global w_accb676a903a06d9
.type w_accb676a903a06d9, @function
w_accb676a903a06d9:
    addi s0, s0, -8          # pop from DS (upward: subtract s0)
    ld a0, 0(s0)             # a0 = value (low byte = char)
    sb a0, (sp)              # store byte on native stack
    addi sp, sp, -16         # make room for semihosting param
    li a0, 0x03              # SYS_WRITEC
    mv a1, sp                # a1 = pointer to byte
    semihost_call
    addi sp, sp, 16          # restore native stack
    ret

# -----------------------------------------------------------------
# testio.write-str ( str -- )
# fnv1a_u64("testio.write-str") = eb06855547211672
#
# str is a pointer to length-prefixed bytes: [u64 len][u8...]
# -----------------------------------------------------------------
.global w_eb06855547211672
.type w_eb06855547211672, @function
w_eb06855547211672:
    addi s0, s0, -8          # pop pointer from DS
    ld t0, 0(s0)             # t0 = pointer to string struct
    ld t1, 0(t0)             # t1 = length (u64)
    addi t0, t0, 8           # t0 = pointer to first byte
    beqz t1, .Lstr_done_rv
.Lstr_loop_rv:
    lbu a0, 0(t0)            # load byte
    sb a0, (sp)
    addi sp, sp, -16         # store on native stack
    li a0, 0x03              # SYS_WRITEC
    mv a1, sp
    semihost_call
    addi sp, sp, 16          # restore
    addi t0, t0, 1           # next byte
    addi t1, t1, -1          # decrement count
    bnez t1, .Lstr_loop_rv
.Lstr_done_rv:
    ret

# -----------------------------------------------------------------
# testio.exit ( i64 -- )
# fnv1a_u64("testio.exit") = f91ca4f233247b4d
#
# Exits via SYS_EXIT with reason ADP_Stopped_ApplicationExit (0x20026).
# QEMU exits with status 0; harness detects pass/fail via S/F markers.
# -----------------------------------------------------------------
.global w_f91ca4f233247b4d
.type w_f91ca4f233247b4d, @function
w_f91ca4f233247b4d:
    addi s0, s0, -8          # pop exit code from DS
    ld a0, 0(s0)             # a0 = exit code
    li a1, 0x20026           # ADP_Stopped_ApplicationExit
    addi sp, sp, -16
    sd a1, 0(sp)             # param block = [reason: u64]
    mv a1, sp
    li a0, 0x18              # SYS_EXIT
    semihost_call
    addi sp, sp, 16          # SYS_EXIT should not return; cleanup if it does
    ret
