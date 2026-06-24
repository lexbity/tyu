# RISC-V RV32 semihosting testio words
# Included by runtime/<triple>/runtime.asm
#
# Expects shared helpers __lang_writec and __lang_fail_exit to be
# defined before the .include point (runtime.asm defines them).
#
# Data-stack discipline (abi-contract 4.4.2):
#   s2 = DS pointer (upward-growing: push = addi s2, +N, pop = addi s2, -N)
#   slot_bytes = 4

# -----------------------------------------------------------------
# testio.write-byte ( i64 -- )
# fnv1a_u64("testio.write-byte") = accb676a903a06d9
#
# i64 occupies two 4-byte DS slots on RV32. Pop both, use low word.
# -----------------------------------------------------------------
.globl w_accb676a903a06d9
.type w_accb676a903a06d9, @function
w_accb676a903a06d9:
    addi sp, sp, -4          # save ra: __lang_writec is reached via jal,
    sw ra, 0(sp)             # which clobbers our own return address
    addi s2, s2, -8          # pop i64 (two DS slots)
    lw a0, 0(s2)             # a0 = low 32 bits (low byte = char)
    jal __lang_writec
    lw ra, 0(sp)
    addi sp, sp, 4
    ret

# -----------------------------------------------------------------
# testio.write-str ( str -- )
# fnv1a_u64("testio.write-str") = eb06855547211672
#
# str is a pointer (4 bytes on RV32) to length-prefixed bytes:
# [u64 len][u8...].
# -----------------------------------------------------------------
.globl w_eb06855547211672
.type w_eb06855547211672, @function
w_eb06855547211672:
    addi sp, sp, -4          # save ra across the __lang_writec calls below
    sw ra, 0(sp)
    addi s2, s2, -4          # pop str pointer (one DS slot)
    lw t0, 0(s2)             # t0 = pointer to string struct
    lw t1, 0(t0)             # t1 = low 32 bits of length
    addi t0, t0, 8           # skip 8-byte u64 length, point to first byte
    beqz t1, .Lstr_done_rv
.Lstr_loop_rv:
    lbu a0, 0(t0)            # load byte
    jal __lang_writec
    addi t0, t0, 1           # next byte
    addi t1, t1, -1          # decrement count
    bnez t1, .Lstr_loop_rv
.Lstr_done_rv:
    lw ra, 0(sp)
    addi sp, sp, 4
    ret

# -----------------------------------------------------------------
# testio.exit ( i64 -- )
# fnv1a_u64("testio.exit") = f91ca4f233247b4d
#
# Pops exit code from DS (discarded), then terminates via SYS_EXIT
# with ADP_Stopped_ApplicationExit (0x20026).
# -----------------------------------------------------------------
.globl w_f91ca4f233247b4d
.type w_f91ca4f233247b4d, @function
w_f91ca4f233247b4d:
    addi s2, s2, -8          # pop i64 (two DS slots)
    lw a0, 0(s2)             # a0 = low 32 bits of exit code (discarded)
    j __lang_fail_exit
