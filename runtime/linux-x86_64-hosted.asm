format ELF64

section '.text' executable
public _start
public __lang_start
public __lang_trap
public __lang_trap_loc
public __stack_overflow
public __task_spawn
public __task_join
public __task_yield
public __task_sleep_ms
public __task_sleep_us
public w_a6b1202e57aa7cc9
public w_eb1d0a3c5e7c2e92
public w_034a1ff17acf93d3

extrn w_1f5962a2ce9803c8 ; main

TASK_WORKERS equ 4
TASK_DEQUE_CAP equ 8
TASK_DEQUE_MASK equ 7
TASK_GLOBAL_CAP equ 16
TASK_GLOBAL_MASK equ 15

_start:
__lang_start:
  mov r15, __lang_ds_base
  mov r14, __lang_ds_limit
  mov qword [__lang_ds_high], r15
  mov qword [__task_current], 0
  mov qword [__task_worker], 0
  mov qword [__task_state], 2
  call w_1f5962a2ce9803c8
  sub r15, 8
  mov rdi, [r15]
  and rdi, 0xff
  mov rax, 60
  syscall

__lang_trap:
__lang_trap_loc:
  mov rax, 60
  syscall

__stack_overflow:
  mov rdi, 10
  jmp __lang_trap

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
  cmp rdx, TASK_DEQUE_CAP
  jae .task_spawn_global
  mov rdx, rax
  and rdx, TASK_DEQUE_MASK
  mov [rsi + rdx*8], r13
  inc rax
  mov [__task_w_tail + rbx*8], rax
  jmp .task_spawn_done
.task_spawn_global:
  mov rax, [__task_g_tail]
  mov rcx, [__task_g_head]
  mov rdx, rax
  sub rdx, rcx
  cmp rdx, TASK_GLOBAL_CAP
  jae .task_spawn_fail
  mov rdx, rax
  and rdx, TASK_GLOBAL_MASK
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

__task_entry_tramp:
  mov rcx, [__task_current]
  mov rax, [__task_entry + rcx*8]
  call rax
  call __task_exit

__task_exit:
  mov rcx, [__task_current]
  mov qword [__task_state + rcx*8], 3
  call __task_yield
  mov rdi, 0
  mov rax, 60
  syscall

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
  cmp rdx, TASK_DEQUE_CAP
  jae .task_yield_enqueue_global
  mov rdx, rax
  and rdx, TASK_DEQUE_MASK
  mov [rsi + rdx*8], rbx
  inc rax
  mov [__task_w_tail + r13*8], rax
  jmp .task_yield_no_enqueue
.task_yield_enqueue_global:
  mov rax, [__task_g_tail]
  mov rcx, [__task_g_head]
  mov rdx, rax
  sub rdx, rcx
  cmp rdx, TASK_GLOBAL_CAP
  jae .task_yield_no_enqueue
  mov rdx, rax
  and rdx, TASK_GLOBAL_MASK
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
  and rdx, TASK_DEQUE_MASK
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
  and rdx, TASK_GLOBAL_MASK
  mov rbx, [__task_g_buf + rdx*8]
  inc rax
  mov [__task_g_head], rax
  mov r10, r13
  jmp .task_yield_switch
.task_yield_steal:
  mov r11, 1
.task_yield_steal_loop:
  cmp r11, TASK_WORKERS
  jae .task_yield_no_ready
  mov r10, r13
  add r10, r11
  cmp r10, TASK_WORKERS
  jb .task_yield_steal_check
  sub r10, TASK_WORKERS
.task_yield_steal_check:
  mov rsi, r10
  shl rsi, 6
  add rsi, __task_w_buf
  mov rax, [__task_w_head + r10*8]
  mov rcx, [__task_w_tail + r10*8]
  cmp rax, rcx
  je .task_yield_steal_next
  mov rdx, rax
  and rdx, TASK_DEQUE_MASK
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
  mov rdi, 23
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

__task_sleep_ms:
  sub rsp, 16
  mov rax, rdi
  xor rdx, rdx
  mov rcx, 1000
  div rcx
  mov [rsp], rax
  mov rax, rdx
  mov rcx, 1000000
  imul rax, rcx
  mov [rsp+8], rax
  mov rdi, rsp
  xor rsi, rsi
  mov rax, 35
  syscall
  add rsp, 16
  call __task_yield
  ret

__task_sleep_us:
  sub rsp, 16
  mov rax, rdi
  xor rdx, rdx
  mov rcx, 1000000
  div rcx
  mov [rsp], rax
  mov rax, rdx
  mov rcx, 1000
  imul rax, rcx
  mov [rsp+8], rax
  mov rdi, rsp
  xor rsi, rsi
  mov rax, 35
  syscall
  add rsp, 16
  call __task_yield
  ret

; ---------------------------------------------------------------------------
; platform.gpio words — synthetic latch for smoke tests
; ---------------------------------------------------------------------------

; platform.gpio.init ( pin mode -- )
;   fnv1a_u64("platform.gpio.init") = a6b1202e57aa7cc9
w_a6b1202e57aa7cc9:
  sub r15, 16
  mov byte [__lang_gpio_state], 0
  ret

; platform.gpio.write ( pin bool -- )
;   fnv1a_u64("platform.gpio.write") = eb1d0a3c5e7c2e92
w_eb1d0a3c5e7c2e92:
  sub r15, 16
  mov al, [r15+8]
  mov byte [__lang_gpio_state], al
  ret

; platform.gpio.read ( pin -- bool )
;   fnv1a_u64("platform.gpio.read") = 034a1ff17acf93d3
w_034a1ff17acf93d3:
  sub r15, 8
  xor rax, rax
  mov al, byte [__lang_gpio_state]
  mov [r15], rax
  add r15, 8
  ret

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
public __region_next
public __region_base
public __region_size
public __region_off
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
public __lang_gpio_state
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
__region_next dq 0
__region_base rq 16
__region_size rq 16
__region_off rq 16
__task_current dq 0
__task_worker dq 0
__task_state rq 16
__task_rsp rq 16
__task_r15 rq 16
__task_r14 rq 16
__task_entry rq 16
__task_w_head rq TASK_WORKERS
__task_w_tail rq TASK_WORKERS
__task_w_buf rq TASK_WORKERS*TASK_DEQUE_CAP
__task_g_head dq 0
__task_g_tail dq 0
__task_g_buf rq TASK_GLOBAL_CAP
__task_ds_mem rb 1048576
__task_cs_mem rb 1048576
__mmio_mem rb 65536
__lang_gpio_state dq 0
__lang_ds_base rb 65536
__lang_ds_limit:
public __lang_ds_high
__lang_ds_high dq 0
public __lang_expected_abi_hash
; compute_abi_hash(ARCH_TAG_X86_64=1, slot=8, word=64, MODINFO_VER=3), recipe v2
__lang_expected_abi_hash dq 0x41f05b8b1adab0ab

; ---------------------------------------------------------------------------
; Module modpack section (S2 Phase 14) — empty for hosted; FS used instead
; ---------------------------------------------------------------------------
section '.modpack' writeable
public __lang_modpack_start
__lang_modpack_start:
public __lang_modpack_end
__lang_modpack_end:
