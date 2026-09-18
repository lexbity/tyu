; ---------------------------------------------------------------------------
; Concurrency runtime unit — linked when the `concurrency` feature is
; enabled.  Provides the task scheduler, channel IPC, and supporting
; BSS (per-task stacks, channel buffers, scheduler state).
;
; Extracted from crates/codegen-x86_64/src/task.rs emit_task_runtime().
; ---------------------------------------------------------------------------

format ELF64

extrn __lang_trap

section '.text' executable
use64

public __task_spawn
public __task_join
public __task_yield
public __task_sleep_ms
public __task_sleep_us
public __task_entry_tramp
public __task_exit

; ===========================================================================
; Task spawn
; ===========================================================================
__task_spawn:
  push rbx
  push r12
  push r13
  mov rbx, 1
.task_spawn_find:
  cmp rbx, 16
  je .task_spawn_fail
  mov r12, [__task_state + rbx*8]
  cmp r12, 0
  je .task_spawn_found
  inc rbx
  jmp .task_spawn_find
.task_spawn_found:
  mov qword [__task_state + rbx*8], 1
  mov [__task_entry + rbx*8], rdi
  mov rax, __task_ds_mem
  mov rcx, rbx
  shl rcx, 16
  add rax, rcx
  mov [__task_r15 + rbx*8], rax
  mov rdx, rax
  add rdx, 65536
  mov [__task_r14 + rbx*8], rdx
  mov rax, __task_cs_mem
  mov rcx, rbx
  shl rcx, 16
  add rax, rcx
  add rax, 65536
  sub rax, 8
  mov qword [rax], __task_entry_tramp
  sub rax, 8
  mov qword [rax], 0
  sub rax, 8
  mov qword [rax], 0
  sub rax, 8
  mov qword [rax], 0
  mov [__task_rsp + rbx*8], rax
  mov r13, rbx
  mov rbx, [__task_worker]
  mov rsi, rbx
  shl rsi, 6
  add rsi, __task_w_buf
  mov rax, [__task_w_tail + rbx*8]
  mov rcx, [__task_w_head + rbx*8]
  mov rdx, rax
  sub rdx, rcx
  cmp rdx, 8
  jae .task_spawn_global
  mov rdx, rax
  and rdx, 7
  mov [rsi + rdx*8], r13
  inc rax
  mov [__task_w_tail + rbx*8], rax
  jmp .task_spawn_done
.task_spawn_global:
  mov rax, [__task_g_tail]
  mov rcx, [__task_g_head]
  mov rdx, rax
  sub rdx, rcx
  cmp rdx, 16
  jae .task_spawn_fail
  mov rdx, rax
  and rdx, 15
  mov [__task_g_buf + rdx*8], r13
  inc rax
  mov [__task_g_tail], rax
.task_spawn_done:
  mov rax, r13
  pop r13
  pop r12
  pop rbx
  ret
.task_spawn_fail:
  pop r13
  pop r12
  pop rbx
  mov rdi, 23
  jmp __lang_trap

; ===========================================================================
; Task entry trampoline
; ===========================================================================
__task_entry_tramp:
  mov rcx, [__task_current]
  mov rax, [__task_entry + rcx*8]
  call rax
  call __task_exit

; ===========================================================================
; Task exit
; ===========================================================================
__task_exit:
  mov rcx, [__task_current]
  mov qword [__task_state + rcx*8], 3
  call __task_yield
  ; Yield returned to a DONE task: nothing is runnable anymore.  Mirror the
  ; hosted runtime's clean exit(0) using this target's exit convention —
  ; isa-debug-exit port 0x501 (guest writes 0 → QEMU exits (0<<1)|1 = pass),
  ; then halt.  A Linux `exit` syscall is invalid here (BUG-015).
  xor ax, ax
  mov dx, 0x501
  out dx, ax
  cli
  hlt

; ===========================================================================
; Task yield
; ===========================================================================
__task_yield:
  push rbx
  push r12
  push r13
  mov rbx, [__task_current]
  mov [__task_rsp + rbx*8], rsp
  mov [__task_r15 + rbx*8], r15
  mov [__task_r14 + rbx*8], r14
  mov r12, [__task_state + rbx*8]
  cmp r12, 0
  jne .task_yield_state_ok
  mov qword [__task_state + rbx*8], 2
  mov r12, 2
.task_yield_state_ok:
  cmp r12, 2
  jne .task_yield_no_enqueue
  mov qword [__task_state + rbx*8], 1
  mov r13, [__task_worker]
  mov rsi, r13
  shl rsi, 6
  add rsi, __task_w_buf
  mov rax, [__task_w_tail + r13*8]
  mov rcx, [__task_w_head + r13*8]
  mov rdx, rax
  sub rdx, rcx
  cmp rdx, 8
  jae .task_yield_enqueue_global
  mov rdx, rax
  and rdx, 7
  mov [rsi + rdx*8], rbx
  inc rax
  mov [__task_w_tail + r13*8], rax
  jmp .task_yield_no_enqueue
.task_yield_enqueue_global:
  mov rax, [__task_g_tail]
  mov rcx, [__task_g_head]
  mov rdx, rax
  sub rdx, rcx
  cmp rdx, 16
  jae .task_yield_no_enqueue
  mov rdx, rax
  and rdx, 15
  mov [__task_g_buf + rdx*8], rbx
  inc rax
  mov [__task_g_tail], rax
.task_yield_no_enqueue:
  mov r13, [__task_worker]
  mov rsi, r13
  shl rsi, 6
  add rsi, __task_w_buf
  mov rax, [__task_w_head + r13*8]
  mov rcx, [__task_w_tail + r13*8]
  cmp rax, rcx
  je .task_yield_try_global
  mov rdx, rax
  and rdx, 7
  mov rbx, [rsi + rdx*8]
  inc rax
  mov [__task_w_head + r13*8], rax
  mov r10, r13
  jmp .task_yield_switch
.task_yield_try_global:
  mov rax, [__task_g_head]
  mov rcx, [__task_g_tail]
  cmp rax, rcx
  je .task_yield_steal
  mov rdx, rax
  and rdx, 15
  mov rbx, [__task_g_buf + rdx*8]
  inc rax
  mov [__task_g_head], rax
  mov r10, r13
  jmp .task_yield_switch
.task_yield_steal:
  mov r11, 1
.task_yield_steal_loop:
  cmp r11, 4
  jae .task_yield_no_ready
  mov r10, r13
  add r10, r11
  cmp r10, 4
  jb .task_yield_steal_check
  sub r10, 4
.task_yield_steal_check:
  mov rsi, r10
  shl rsi, 6
  add rsi, __task_w_buf
  mov rax, [__task_w_head + r10*8]
  mov rcx, [__task_w_tail + r10*8]
  cmp rax, rcx
  je .task_yield_steal_next
  mov rdx, rax
  and rdx, 7
  mov rbx, [rsi + rdx*8]
  inc rax
  mov [__task_w_head + r10*8], rax
  jmp .task_yield_switch
.task_yield_steal_next:
  inc r11
  jmp .task_yield_steal_loop
.task_yield_no_ready:
  cmp r12, 2
  je .task_yield_no_ready_active
  cmp r12, 4
  jne .task_yield_return
  ; All tasks blocked and nothing is runnable: program deadlock (BUG-005) —
  ; deliver Deadlock (25), not the generic Unreachable (23).
  mov rdi, 25
  jmp __lang_trap
.task_yield_no_ready_active:
  mov qword [__task_state + rbx*8], 2
.task_yield_return:
  pop r13
  pop r12
  pop rbx
  ret
.task_yield_switch:
  mov qword [__task_state + rbx*8], 2
  mov [__task_current], rbx
  mov [__task_worker], r10
  mov rsp, [__task_rsp + rbx*8]
  mov r15, [__task_r15 + rbx*8]
  mov r14, [__task_r14 + rbx*8]
  pop r13
  pop r12
  pop rbx
  ret

; ===========================================================================
; Task join
; ===========================================================================
__task_join:
  push rbx
  mov rbx, rdi
  cmp rbx, 16
  jb .task_join_loop
  mov rdi, 23
  jmp __lang_trap
.task_join_loop:
  mov rax, [__task_state + rbx*8]
  cmp rax, 3
  je .task_join_done
  call __task_yield
  jmp .task_join_loop
.task_join_done:
  mov qword [__task_state + rbx*8], 0
  pop rbx
  ret

; ===========================================================================
; Task sleep (milliseconds)
; ===========================================================================
; Bare metal has no timer driver, and the sysroot exposes sleep only on the
; hosted target — nothing in the language surface can reach these entry
; points.  A direct call must fail loudly instead of executing an invalid
; Linux nanosleep syscall (BUG-015).
__task_sleep_ms:
  mov rdi, 23
  jmp __lang_trap

; ===========================================================================
; Task sleep (microseconds)
; ===========================================================================
__task_sleep_us:
  mov rdi, 23
  jmp __lang_trap

; ===========================================================================
; BSS — scheduler and channel state
; ===========================================================================
section '.bss' writeable

public __chan_next
public __chan_inuse
public __chan_head
public __chan_tail
public __chan_buf
public __chan_wait_recv_head
public __chan_wait_recv_tail
public __chan_wait_recv_buf
public __chan_wait_send_head
public __chan_wait_send_tail
public __chan_wait_send_buf
public __task_current
public __task_worker
public __task_state
public __task_rsp
public __task_r15
public __task_r14
public __task_entry
public __task_w_head
public __task_w_tail
public __task_w_buf
public __task_g_head
public __task_g_tail
public __task_g_buf
public __task_ds_mem
public __task_cs_mem

__chan_next dq 0
__chan_inuse rq 16
__chan_head rq 16
__chan_tail rq 16
__chan_buf rq 1024
__chan_wait_recv_head rq 16
__chan_wait_recv_tail rq 16
__chan_wait_recv_buf rq 128
__chan_wait_send_head rq 16
__chan_wait_send_tail rq 16
__chan_wait_send_buf rq 128
__task_current dq 0
__task_worker dq 0
__task_state rq 16
__task_rsp rq 16
__task_r15 rq 16
__task_r14 rq 16
__task_entry rq 16
__task_w_head rq 4
__task_w_tail rq 4
__task_w_buf rq 32
__task_g_head dq 0
__task_g_tail dq 0
__task_g_buf rq 16
__task_ds_mem rb 1048576
__task_cs_mem rb 1048576
