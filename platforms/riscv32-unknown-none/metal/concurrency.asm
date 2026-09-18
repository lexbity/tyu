# ---------------------------------------------------------------------------
# Concurrency runtime unit for riscv32-unknown-none (RV32IM).
# Linked when the `concurrency` feature is enabled.
#
# Register convention:
#   s2 = data-stack pointer (upward-growing, preserved across yield)
#   s3 = data-stack limit
#   a0-a1 = value low/high words
#   a2-a4 = scratch
#   t0-t2 = scratch
#   sp   = native call stack
#   ra   = return address
# ---------------------------------------------------------------------------

.section .text
.globl __task_spawn
.globl __task_entry_tramp
.globl __task_exit
.globl __task_yield
.globl __task_join

# ===========================================================================
# __task_spawn ( a0 = entry function pointer ) -> a0 = task_id
# ===========================================================================
__task_spawn:
    addi sp, sp, -16
    sw ra, 12(sp)
    sw s0, 8(sp)
    sw s1, 4(sp)
    sw s4, 0(sp)
    mv s4, a0                 # s4 = entry point
    li s0, 1                  # start search from slot 1
.spawn_find:
    li t0, 16
    bge s0, t0, .spawn_fail
    la t0, __task_state
    slli t1, s0, 2
    add t0, t0, t1
    lw t0, 0(t0)
    beqz t0, .spawn_found
    addi s0, s0, 1
    j .spawn_find
.spawn_found:
    # mark slot READY (1)
    la t0, __task_state
    slli t1, s0, 2
    add t0, t0, t1
    li t1, 1
    sw t1, 0(t0)
    # set entry point
    la t0, __task_entry
    slli t1, s0, 2
    add t0, t0, t1
    sw s4, 0(t0)
    # set DS pointer: __task_ds_mem + task_id * 65536
    la t0, __task_ds_mem
    slli t1, s0, 16
    add s1, t0, t1            # s1 = DS base for this task
    la t0, __task_r15
    slli t1, s0, 2
    add t0, t0, t1
    sw s1, 0(t0)
    # set DS limit: base + 65536
    li t6, 65536
    add s1, s1, t6
    la t0, __task_r14
    slli t1, s0, 2
    add t0, t0, t1
    sw s1, 0(t0)
    # set up initial call stack in __task_cs_mem
    la t0, __task_cs_mem
    slli t1, s0, 16
    add t0, t0, t1
    li t6, 65536
    add t0, t0, t6
    addi t0, t0, -16          # space for trampoline + 3 saved regs
    la t1, __task_entry_tramp
    sw t1, 12(t0)
    sw zero, 8(t0)
    sw zero, 4(t0)
    sw zero, 0(t0)
    la t1, __task_rsp
    slli t2, s0, 2
    add t1, t1, t2
    sw t0, 0(t1)
    # enqueue to current worker's local queue
    la t0, __task_worker
    lw s1, 0(t0)              # s1 = current worker
    la t0, __task_w_buf
    slli t1, s1, 3            # each worker: 8 slots * 4 bytes = 32 bytes
    add t0, t0, t1
    la t1, __task_w_tail
    slli t2, s1, 2
    add t1, t1, t2
    lw t2, 0(t1)              # t2 = tail
    la t3, __task_w_head
    slli t4, s1, 2
    add t3, t3, t4
    lw t3, 0(t3)              # t3 = head
    sub t4, t2, t3
    li t5, 8
    bge t4, t5, .spawn_global
    slli t4, t2, 2
    add t4, t0, t4
    sw s0, 0(t4)              # store task_id in queue
    addi t2, t2, 1
    sw t2, 0(t1)              # update tail
    j .spawn_done
.spawn_global:
    la t0, __task_g_buf
    la t1, __task_g_tail
    lw t2, 0(t1)
    la t3, __task_g_head
    lw t3, 0(t3)
    sub t4, t2, t3
    li t5, 16
    bge t4, t5, .spawn_fail
    slli t4, t2, 2
    add t4, t0, t4
    sw s0, 0(t4)
    addi t2, t2, 1
    sw t2, 0(t1)
.spawn_done:
    mv a0, s0                 # return task_id
    lw ra, 12(sp)
    lw s0, 8(sp)
    lw s1, 4(sp)
    lw s4, 0(sp)
    addi sp, sp, 16
    ret
.spawn_fail:
    # No free slot or global queue full: trap (BUG-005) instead of returning
    # task id 0 (which would alias main).
    lw ra, 12(sp)
    lw s0, 8(sp)
    lw s1, 4(sp)
    lw s4, 0(sp)
    addi sp, sp, 16
    li a0, 23
    j __lang_trap

# ===========================================================================
# __task_entry_tramp
# ===========================================================================
__task_entry_tramp:
    addi sp, sp, -4
    sw ra, 0(sp)
    la t0, __task_current
    lw t0, 0(t0)
    la t1, __task_entry
    slli t2, t0, 2
    add t1, t1, t2
    lw t1, 0(t1)
    jalr t1
    jal __task_exit
    lw ra, 0(sp)
    addi sp, sp, 4
    ret

# ===========================================================================
# __task_exit
# ===========================================================================
__task_exit:
    addi sp, sp, -4
    sw ra, 0(sp)
    la t0, __task_current
    lw t0, 0(t0)
    la t1, __task_state
    slli t2, t0, 2
    add t1, t1, t2
    li t2, 3
    sw t2, 0(t1)
    jal __task_yield
    unimp                   # should not return
    lw ra, 0(sp)
    addi sp, sp, 4
    ret

# ===========================================================================
# __task_yield
#   Saves current context (s2=DS ptr, s3=DS limit, sp), re-enqueues if
#   ACTIVE, then finds next task: local queue → global queue → stay.
# ===========================================================================
__task_yield:
    addi sp, sp, -24
    sw ra, 20(sp)
    sw s0, 16(sp)
    sw s1, 12(sp)
    sw s4, 8(sp)
    sw s5, 4(sp)
    sw s6, 0(sp)
    # save current task context
    la t0, __task_current
    lw s0, 0(t0)              # s0 = current task id
    la t0, __task_rsp
    slli t1, s0, 2
    add t0, t0, t1
    sw sp, 0(t0)
    la t0, __task_r15
    slli t1, s0, 2
    add t0, t0, t1
    sw s2, 0(t0)              # save DS pointer
    la t0, __task_r14
    slli t1, s0, 2
    add t0, t0, t1
    sw s3, 0(t0)              # save DS limit
    # capture entry state for the no-ready/deadlock decision (BUG-005)
    la t0, __task_state
    slli t1, s0, 2
    add t0, t0, t1
    lw s6, 0(t0)              # s6 = entry state
    # re-enqueue if state == ACTIVE (2)
    li t2, 2
    bne s6, t2, .yield_no_enqueue
    li t1, 1                  # READY
    sw t1, 0(t0)
    # enqueue to current worker
    la t0, __task_worker
    lw s5, 0(t0)              # s5 = worker
    la t0, __task_w_buf
    slli t1, s5, 3
    add t0, t0, t1
    la t1, __task_w_tail
    slli t2, s5, 2
    add t1, t1, t2
    lw t2, 0(t1)
    la t3, __task_w_head
    slli t4, s5, 2
    add t3, t3, t4
    lw t3, 0(t3)
    sub t4, t2, t3
    li t5, 8
    bge t4, t5, .yield_enqueue_global
    slli t4, t2, 2
    add t4, t0, t4
    sw s0, 0(t4)
    addi t2, t2, 1
    sw t2, 0(t1)
    j .yield_no_enqueue
.yield_enqueue_global:
    la t0, __task_g_buf
    la t1, __task_g_tail
    lw t2, 0(t1)
    la t3, __task_g_head
    lw t3, 0(t3)
    sub t4, t2, t3
    li t5, 16
    bge t4, t5, .yield_no_enqueue
    slli t4, t2, 2
    add t4, t0, t4
    sw s0, 0(t4)
    addi t2, t2, 1
    sw t2, 0(t1)
.yield_no_enqueue:
    # find next task: local queue first
    la t0, __task_worker
    lw s5, 0(t0)
    la t0, __task_w_buf
    slli t1, s5, 3
    add t0, t0, t1
    la t1, __task_w_head
    slli t2, s5, 2
    add t1, t1, t2
    lw t2, 0(t1)              # head
    la t3, __task_w_tail
    slli t4, s5, 2
    add t3, t3, t4
    lw t3, 0(t3)              # tail
    bne t2, t3, .yield_dequeue_local
    # global queue
    la t0, __task_g_buf
    la t1, __task_g_head
    lw t2, 0(t1)
    la t3, __task_g_tail
    lw t3, 0(t3)
    bne t2, t3, .yield_dequeue_global
    # nothing ready — keep running current task
    j .yield_return_self
.yield_dequeue_local:
    slli t4, t2, 2
    add t4, t0, t4
    lw s0, 0(t4)              # s0 = new task id
    addi t2, t2, 1
    sw t2, 0(t1)
    j .yield_switch
.yield_dequeue_global:
    slli t4, t2, 2
    add t4, t0, t4
    lw s0, 0(t4)
    addi t2, t2, 1
    sw t2, 0(t1)
    j .yield_switch
.yield_return_self:
    # deadlock detection (BUG-005): nothing is runnable and the current task
    # was BLOCKED (state 4) → the program is deadlocked — deliver Deadlock
    # (25), not the generic Unreachable (23).
    li t0, 2
    beq s6, t0, .yield_keep_running
    li t0, 4
    bne s6, t0, .yield_keep_running
    lw ra, 20(sp)
    lw s0, 16(sp)
    lw s1, 12(sp)
    lw s4, 8(sp)
    lw s5, 4(sp)
    lw s6, 0(sp)
    addi sp, sp, 24
    li a0, 25
    j __lang_trap
.yield_keep_running:
    lw ra, 20(sp)
    lw s0, 16(sp)
    lw s1, 12(sp)
    lw s4, 8(sp)
    lw s5, 4(sp)
    lw s6, 0(sp)
    addi sp, sp, 24
    ret
.yield_switch:
    # switch to task s0
    la t0, __task_state
    slli t1, s0, 2
    add t0, t0, t1
    li t1, 2
    sw t1, 0(t0)              # state = ACTIVE
    la t0, __task_current
    sw s0, 0(t0)
    la t0, __task_rsp
    slli t1, s0, 2
    add t0, t0, t1
    lw sp, 0(t0)
    la t0, __task_r15
    slli t1, s0, 2
    add t0, t0, t1
    lw s2, 0(t0)              # restore DS pointer
    la t0, __task_r14
    slli t1, s0, 2
    add t0, t0, t1
    lw s3, 0(t0)              # restore DS limit
    lw ra, 20(sp)
    lw s0, 16(sp)
    lw s1, 12(sp)
    lw s4, 8(sp)
    lw s5, 4(sp)
    lw s6, 0(sp)
    addi sp, sp, 24
    ret

# ===========================================================================
# __task_join ( a0 = task_id )
# ===========================================================================
__task_join:
    addi sp, sp, -8
    sw ra, 4(sp)
    sw s0, 0(sp)
    mv s0, a0
    li t0, 16
    bgeu s0, t0, .join_invalid
.join_loop:
    la t0, __task_state
    slli t1, s0, 2
    add t0, t0, t1
    lw t0, 0(t0)
    li t1, 3
    beq t0, t1, .join_done
    jal __task_yield
    j .join_loop
.join_done:
    la t0, __task_state
    slli t1, s0, 2
    add t0, t0, t1
    sw zero, 0(t0)
.join_invalid:
    # Invalid task id: trap (BUG-005) instead of silently returning.
    lw ra, 4(sp)
    lw s0, 0(sp)
    addi sp, sp, 8
    li a0, 23
    j __lang_trap

# ===========================================================================
# BSS
# ===========================================================================
.section .bss

.globl __task_current
.globl __task_worker
.globl __task_state
.globl __task_rsp
.globl __task_r15
.globl __task_r14
.globl __task_entry
.globl __task_w_head
.globl __task_w_tail
.globl __task_w_buf
.globl __task_g_head
.globl __task_g_tail
.globl __task_g_buf
.globl __task_ds_mem
.globl __task_cs_mem

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
__task_ds_mem:  .space 1048576
__task_cs_mem:  .space 1048576
