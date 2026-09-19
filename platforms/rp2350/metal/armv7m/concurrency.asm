@ ---------------------------------------------------------------------------
@ Concurrency runtime unit for armv7m-unknown-none (Cortex-M3 Thumb-2).
@ Linked when the `concurrency` feature is enabled.
@
@ Register convention:
@   r0-r3  = scratch / args
@   r4     = data-stack pointer (preserved across yield)
@   r5     = data-stack limit (preserved across yield)
@   r6-r11 = callee-saved / scratch
@   r12    = intra-procedure scratch
@   sp     = native call stack (downward-growing)
@   lr     = link register
@ ---------------------------------------------------------------------------

.syntax unified
.thumb

.section .text, "ax"
.thumb_func

.global __task_spawn
.global __task_entry_tramp
.global __task_exit
.global __task_yield
.global __task_join

@ ===========================================================================
@ __task_spawn ( r0 = entry )
@   Searches for a free task slot (1..15, 0 = main), sets up DS/CS memory and
@   initial call stack, marks READY, enqueues to current worker, returns task ID.
@ ===========================================================================
__task_spawn:
    push {r4, r5, r6, r7, lr}
    mov r6, r0                 @ r6 = entry point
    mov r7, #1                 @ start search from slot 1
.spawn_find:
    cmp r7, #6                 @ max 6 task slots (0 = main, 1..5 workers) on 64 KB SRAM
    beq .spawn_fail
    ldr r0, =__task_state
    ldr r0, [r0, r7, lsl #2]
    cmp r0, #0
    beq .spawn_found
    add r7, r7, #1
    b .spawn_find
.spawn_found:
    @ slot r7 is free — mark READY (1)
    ldr r0, =__task_state
    mov r1, #1
    str r1, [r0, r7, lsl #2]
    @ set entry
    ldr r0, =__task_entry
    str r6, [r0, r7, lsl #2]
    @ set DS pointer: __task_ds_mem + task_id * 2048 (2 KiB per-task slice)
    ldr r0, =__task_ds_mem
    mov r1, r7
    lsl r1, r1, #11            @ task_id * 2 KB (per-task stack stride, 64 KB SRAM)
    add r0, r0, r1
    ldr r1, =__task_r15
    str r0, [r1, r7, lsl #2]
    @ set DS limit: base + 2048
    add r0, r0, #2048          @ per-task stack size = 2 KB
    ldr r1, =__task_r14
    str r0, [r1, r7, lsl #2]
    @ set up initial call stack in __task_cs_mem
    ldr r0, =__task_cs_mem
    mov r1, r7
    lsl r1, r1, #11            @ task_id * 2 KB (per-task stack stride, 64 KB SRAM)
    add r0, r0, r1
    add r0, r0, #2048          @ per-task stack size = 2 KB
    sub r0, r0, #16           @ space for trampoline + 3 zeros
    ldr r1, =__task_entry_tramp
    str r1, [r0, #12]
    mov r1, #0
    str r1, [r0, #8]
    str r1, [r0, #4]
    str r1, [r0, #0]
    ldr r1, =__task_rsp
    str r0, [r1, r7, lsl #2]
    @ enqueue to current worker's local queue
    ldr r0, =__task_worker
    ldr r6, [r0]               @ r6 = current worker
    ldr r0, =__task_w_buf
    mov r1, r6
    lsl r1, r1, #3             @ each worker has 8 slots (32 bytes)
    add r0, r0, r1             @ r0 = &__task_w_buf[worker]
    ldr r1, =__task_w_tail
    ldr r2, [r1, r6, lsl #2]   @ r2 = tail
    ldr r3, =__task_w_head
    ldr r3, [r3, r6, lsl #2]   @ r3 = head
    mov r12, r2
    sub r12, r12, r3
    cmp r12, #8
    bhs .spawn_global
    lsl r3, r2, #2
    str r7, [r0, r3]
    add r2, r2, #1
    str r2, [r1, r6, lsl #2]
    b .spawn_done
.spawn_global:
    ldr r0, =__task_g_buf
    ldr r1, =__task_g_tail
    ldr r2, [r1]
    ldr r3, =__task_g_head
    ldr r3, [r3]
    mov r12, r2
    sub r12, r12, r3
    cmp r12, #16
    bhs .spawn_fail_full
    lsl r3, r2, #2
    str r7, [r0, r3]
    add r2, r2, #1
    str r2, [r1]
.spawn_done:
    mov r0, r7                 @ return task ID
    pop {r4, r5, r6, r7, pc}
.spawn_fail:
.spawn_fail_full:
    @ No free slot or global queue full: trap (BUG-005) instead of returning
    @ task id 0 (which would alias main).
    pop {r4, r5, r6, r7, lr}
    movs r0, #23
    b __lang_trap

@ ===========================================================================
@ __task_entry_tramp
@ ===========================================================================
.thumb_func
__task_entry_tramp:
    push {lr}
    ldr r0, =__task_current
    ldr r0, [r0]
    ldr r1, =__task_entry
    ldr r1, [r1, r0, lsl #2]
    blx r1
    bl __task_exit
    pop {pc}

@ ===========================================================================
@ __task_exit
@ ===========================================================================
.thumb_func
__task_exit:
    push {lr}
    ldr r0, =__task_current
    ldr r0, [r0]
    ldr r1, =__task_state
    mov r2, #3
    str r2, [r1, r0, lsl #2]
    bl __task_yield
    bkpt #0
    pop {pc}

@ ===========================================================================
@ __task_yield
@   Saves current task context (r4=DS ptr, r5=DS limit, sp, state),
@   re-enqueues if still ACTIVE, then finds next task: local queue,
@   global queue, or returns immediately.
@ ===========================================================================
.thumb_func
__task_yield:
    push {r4, r5, r6, r7, r8, lr}
    @ --- save current task context ---
    ldr r0, =__task_current
    ldr r6, [r0]               @ r6 = current task ID
    ldr r0, =__task_rsp
    str sp, [r0, r6, lsl #2]
    ldr r0, =__task_r15
    str r4, [r0, r6, lsl #2]   @ save DS pointer (r4)
    ldr r0, =__task_r14
    str r5, [r0, r6, lsl #2]   @ save DS limit (r5)
    @ --- capture entry state for the no-ready/deadlock decision (BUG-005) ---
    ldr r0, =__task_state
    ldr r8, [r0, r6, lsl #2]   @ r8 = entry state
    @ --- re-enqueue if state == ACTIVE (2) ---
    cmp r8, #2
    bne .yield_no_enqueue
    mov r7, #1                 @ READY
    str r7, [r0, r6, lsl #2]
    @ enqueue to current worker
    ldr r0, =__task_worker
    ldr r7, [r0]               @ r7 = worker
    ldr r0, =__task_w_buf
    mov r1, r7
    lsl r1, r1, #3
    add r0, r0, r1
    ldr r1, =__task_w_tail
    ldr r2, [r1, r7, lsl #2]
    ldr r3, =__task_w_head
    ldr r3, [r3, r7, lsl #2]
    mov r12, r2
    sub r12, r12, r3
    cmp r12, #8
    bhs .yield_enqueue_global
    lsl r3, r2, #2
    str r6, [r0, r3]
    add r2, r2, #1
    str r2, [r1, r7, lsl #2]
    b .yield_no_enqueue
.yield_enqueue_global:
    ldr r0, =__task_g_buf
    ldr r1, =__task_g_tail
    ldr r2, [r1]
    ldr r3, =__task_g_head
    ldr r3, [r3]
    mov r12, r2
    sub r12, r12, r3
    cmp r12, #16
    bhs .yield_no_enqueue
    lsl r3, r2, #2
    str r6, [r0, r3]
    add r2, r2, #1
    str r2, [r1]
.yield_no_enqueue:
    @ --- find next runnable task ---
    ldr r0, =__task_worker
    ldr r7, [r0]               @ r7 = current worker
    @ local queue
    ldr r0, =__task_w_buf
    mov r1, r7
    lsl r1, r1, #3
    add r0, r0, r1
    ldr r1, =__task_w_head
    ldr r2, [r1, r7, lsl #2]
    ldr r3, =__task_w_tail
    ldr r3, [r3, r7, lsl #2]
    cmp r2, r3
    bne .yield_dequeue_local
    @ global queue
    ldr r0, =__task_g_buf
    ldr r1, =__task_g_head
    ldr r2, [r1]
    ldr r3, =__task_g_tail
    ldr r3, [r3]
    cmp r2, r3
    bne .yield_dequeue_global
    @ nothing ready — keep running current task
    b .yield_return_self
.yield_dequeue_local:
    lsl r3, r2, #2
    ldr r6, [r0, r3]
    add r2, r2, #1
    str r2, [r1, r7, lsl #2]
    b .yield_switch
.yield_dequeue_global:
    lsl r3, r2, #2
    ldr r6, [r0, r3]
    add r2, r2, #1
    str r2, [r1]
    b .yield_switch
.yield_return_self:
    @ deadlock detection (BUG-005): nothing is runnable and the current task
    @ was BLOCKED (state 4) → the program is deadlocked — deliver Deadlock
    @ (25), not the generic Unreachable (23).
    cmp r8, #2
    beq .yield_keep_running
    cmp r8, #4
    bne .yield_keep_running
    pop {r4, r5, r6, r7, r8}
    movs r0, #25
    b __lang_trap
.yield_keep_running:
    @ stay on current task
    pop {r4, r5, r6, r7, r8, pc}
.yield_switch:
    @ switch to task r6
    ldr r0, =__task_state
    mov r1, #2
    str r1, [r0, r6, lsl #2]    @ state = ACTIVE (2)
    ldr r0, =__task_current
    str r6, [r0]
    ldr r0, =__task_rsp
    ldr sp, [r0, r6, lsl #2]
    ldr r0, =__task_r15
    ldr r4, [r0, r6, lsl #2]    @ restore DS pointer
    ldr r0, =__task_r14
    ldr r5, [r0, r6, lsl #2]    @ restore DS limit
    pop {r4, r5, r6, r7, r8, pc}

@ ===========================================================================
@ __task_join ( r0 = task_id )
@   Waits until task exits (state == 3), then frees slot.
@ ===========================================================================
.thumb_func
__task_join:
    push {r4, lr}
    mov r4, r0
    cmp r4, #6                 @ max 6 task slots
    bhs .join_invalid
.join_loop:
    ldr r0, =__task_state
    ldr r0, [r0, r4, lsl #2]
    cmp r0, #3
    beq .join_done
    bl __task_yield
    b .join_loop
.join_done:
    ldr r0, =__task_state
    mov r1, #0
    str r1, [r0, r4, lsl #2]
.join_invalid:
    @ Invalid task id: trap (BUG-005) instead of silently returning.
    pop {r4}
    movs r0, #23
    b __lang_trap

@ ===========================================================================
@ BSS
@ ===========================================================================
.section .bss, "aw", %nobits

.global __task_current
.global __task_worker
.global __task_state
.global __task_rsp
.global __task_r15
.global __task_r14
.global __task_entry
.global __task_w_head
.global __task_w_tail
.global __task_w_buf
.global __task_g_head
.global __task_g_tail
.global __task_g_buf
.global __task_ds_mem
.global __task_cs_mem

__task_current: .space 4
__task_worker:  .space 4
__task_state:   .space 64
__task_rsp:     .space 64
__task_r15:     .space 64
__task_r14:     .space 64
__task_entry:   .space 64
__task_w_head:  .space 16
__task_w_tail:  .space 16
__task_w_buf:   .space 128
__task_g_head:  .space 4
__task_g_tail:  .space 4
__task_g_buf:   .space 64
@ 8 task slots × 2 KB per-task stack = 16 KB each (fits 64 KB SRAM alongside
@ the main 16 KB data stack and the downward-growing native stack).
__task_ds_mem:  .space 12288
__task_cs_mem:  .space 12288
