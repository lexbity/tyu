@ ARM Thumb semihosting testio words
@ Included by runtime/<triple>/runtime.asm
@
@ Expects shared helpers __lang_writec and __lang_fail_exit to be
@ defined before the .include point (runtime.asm defines them).
@
@ Semihosting convention (ARM):
@   r0 = operation number, r1 = parameter block pointer, bkpt 0xAB
@
@ Data-stack discipline (abi-contract 4.4.2):
@   r4 = DS pointer (upward-growing: push = adds r4, pop = subs r4)
@   slot_bytes = 4

.syntax unified
.thumb

@ -----------------------------------------------------------------
@ testio.write-byte ( i64 -- )
@ fnv1a_u64("testio.write-byte") = accb676a903a06d9
@ -----------------------------------------------------------------
.global w_accb676a903a06d9
.type w_accb676a903a06d9, %function
w_accb676a903a06d9:
    subs r4, r4, #8          @ pop i64 (two DS slots)
    ldr r0, [r4]             @ r0 = low 32 bits of i64 (low byte = char)
    bl __lang_writec
    bx lr

@ -----------------------------------------------------------------
@ testio.write-str ( str -- )
@ fnv1a_u64("testio.write-str") = eb06855547211672
@
@ str is a pointer (4 bytes on ARM) to length-prefixed bytes:
@ [u64 len][u8...]. The u64 length is always 8 bytes regardless
@ of register width.
@ -----------------------------------------------------------------
.global w_eb06855547211672
.type w_eb06855547211672, %function
w_eb06855547211672:
    subs r4, r4, #4          @ pop pointer from DS
    ldr r2, [r4]             @ r2 = pointer to string struct
    ldr r3, [r2]             @ r3 = low 32 bits of length
    adds r2, r2, #8          @ skip 8-byte u64 length, point to first byte
    cbz r3, .Lstr_done_arm
.Lstr_loop_arm:
    ldrb r0, [r2]            @ load byte
    bl __lang_writec
    adds r2, r2, #1          @ next byte
    subs r3, r3, #1          @ decrement count
    bne .Lstr_loop_arm
.Lstr_done_arm:
    bx lr

@ -----------------------------------------------------------------
@ testio.exit ( i64 -- )
@ fnv1a_u64("testio.exit") = f91ca4f233247b4d
@
@ Pops exit code from DS (discarded), then terminates via SYS_EXIT
@ with ADP_Stopped_ApplicationExit (0x20026).  QEMU exits with
@ status 0; harness detects pass/fail via S/F serial markers.
@ -----------------------------------------------------------------
.global w_f91ca4f233247b4d
.type w_f91ca4f233247b4d, %function
w_f91ca4f233247b4d:
    subs r4, r4, #8          @ pop i64 (two DS slots)
    ldr r0, [r4]             @ r0 = low 32 bits of exit code (discarded)
    b __lang_fail_exit
