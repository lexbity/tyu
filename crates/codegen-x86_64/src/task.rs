use ir as lir;

use crate::ophelpers;
use crate::util::write_u32;
use crate::X86_64HostedBackend;

pub fn emit_task_spawn(gen: &mut X86_64HostedBackend<'_>, name: &[u8]) {
    gen.out.write(b"  mov rdi, ");
    crate::ophelpers::write_label(gen.out, name);
    gen.out.write(b"\n");
    gen.out.write(b"  call __task_spawn\n");
    ophelpers::emit_push_rax(gen.out);
}

pub fn emit_task_yield(gen: &mut X86_64HostedBackend<'_>) {
    gen.out.write(b"  call __task_yield\n");
}

pub fn emit_task_join(gen: &mut X86_64HostedBackend<'_>) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rdi, [r15]\n");
    gen.out.write(b"  call __task_join\n");
}

pub fn emit_task_sleep_ms(gen: &mut X86_64HostedBackend<'_>) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rdi, [r15]\n");
    gen.out.write(b"  call __task_sleep_ms\n");
}

pub fn emit_task_sleep_us(gen: &mut X86_64HostedBackend<'_>) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rdi, [r15]\n");
    gen.out.write(b"  call __task_sleep_us\n");
}

fn emit_task_spawn_runtime(out: &mut dyn frontend::parse::Output) {
    out.write(b"\n__task_spawn:\n");
    out.write(b"  push rbx\n");
    out.write(b"  push r12\n");
    out.write(b"  push r13\n");
    out.write(b"  mov rbx, 1\n");
    out.write(b".task_spawn_find:\n");
    out.write(b"  cmp rbx, 16\n");
    out.write(b"  je .task_spawn_fail\n");
    out.write(b"  mov r12, [__task_state + rbx*8]\n");
    out.write(b"  cmp r12, 0\n");
    out.write(b"  je .task_spawn_found\n");
    out.write(b"  inc rbx\n");
    out.write(b"  jmp .task_spawn_find\n");
    out.write(b".task_spawn_found:\n");
    out.write(b"  mov qword [__task_state + rbx*8], 1\n");
    out.write(b"  mov [__task_entry + rbx*8], rdi\n");
    out.write(b"  mov rax, __task_ds_mem\n");
    out.write(b"  mov rcx, rbx\n");
    out.write(b"  shl rcx, 16\n");
    out.write(b"  add rax, rcx\n");
    out.write(b"  mov [__task_r15 + rbx*8], rax\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  add rdx, 65536\n");
    out.write(b"  mov [__task_r14 + rbx*8], rdx\n");
    out.write(b"  mov rax, __task_cs_mem\n");
    out.write(b"  mov rcx, rbx\n");
    out.write(b"  shl rcx, 16\n");
    out.write(b"  add rax, rcx\n");
    out.write(b"  add rax, 65536\n");
    out.write(b"  sub rax, 8\n");
    out.write(b"  mov qword [rax], __task_entry_tramp\n");
    out.write(b"  sub rax, 8\n");
    out.write(b"  mov qword [rax], 0\n");
    out.write(b"  sub rax, 8\n");
    out.write(b"  mov qword [rax], 0\n");
    out.write(b"  sub rax, 8\n");
    out.write(b"  mov qword [rax], 0\n");
    out.write(b"  mov [__task_rsp + rbx*8], rax\n");
    out.write(b"  mov r13, rbx\n");
    out.write(b"  mov rbx, [__task_worker]\n");
    out.write(b"  mov rsi, rbx\n");
    out.write(b"  shl rsi, 6\n");
    out.write(b"  add rsi, __task_w_buf\n");
    out.write(b"  mov rax, [__task_w_tail + rbx*8]\n");
    out.write(b"  mov rcx, [__task_w_head + rbx*8]\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  sub rdx, rcx\n");
    out.write(b"  cmp rdx, 8\n");
    out.write(b"  jae .task_spawn_global\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  and rdx, 7\n");
    out.write(b"  mov [rsi + rdx*8], r13\n");
    out.write(b"  inc rax\n");
    out.write(b"  mov [__task_w_tail + rbx*8], rax\n");
    out.write(b"  jmp .task_spawn_done\n");
    out.write(b".task_spawn_global:\n");
    out.write(b"  mov rax, [__task_g_tail]\n");
    out.write(b"  mov rcx, [__task_g_head]\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  sub rdx, rcx\n");
    out.write(b"  cmp rdx, 16\n");
    out.write(b"  jae .task_spawn_fail\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  and rdx, 15\n");
    out.write(b"  mov [__task_g_buf + rdx*8], r13\n");
    out.write(b"  inc rax\n");
    out.write(b"  mov [__task_g_tail], rax\n");
    out.write(b".task_spawn_done:\n");
    out.write(b"  mov rax, r13\n");
    out.write(b"  pop r13\n");
    out.write(b"  pop r12\n");
    out.write(b"  pop rbx\n");
    out.write(b"  ret\n");
    out.write(b".task_spawn_fail:\n");
    out.write(b"  pop r13\n");
    out.write(b"  pop r12\n");
    out.write(b"  pop rbx\n");
    out.write(b"  mov rdi, ");
    write_u32(out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    out.write(b"\n");
    out.write(b"  jmp __lang_trap\n");
}

fn emit_task_entry_exit_runtime(out: &mut dyn frontend::parse::Output) {
    out.write(b"\n__task_entry_tramp:\n");
    out.write(b"  mov rcx, [__task_current]\n");
    out.write(b"  mov rax, [__task_entry + rcx*8]\n");
    out.write(b"  call rax\n");
    out.write(b"  call __task_exit\n");

    out.write(b"\n__task_exit:\n");
    out.write(b"  mov rcx, [__task_current]\n");
    out.write(b"  mov qword [__task_state + rcx*8], 3\n");
    out.write(b"  call __task_yield\n");
    out.write(b"  mov rdi, 0\n");
    out.write(b"  mov rax, 60\n");
    out.write(b"  syscall\n");
}

fn emit_task_yield_runtime(out: &mut dyn frontend::parse::Output) {
    out.write(b"\n__task_yield:\n");
    out.write(b"  push rbx\n");
    out.write(b"  push r12\n");
    out.write(b"  push r13\n");
    out.write(b"  mov rbx, [__task_current]\n");
    out.write(b"  mov [__task_rsp + rbx*8], rsp\n");
    out.write(b"  mov [__task_r15 + rbx*8], r15\n");
    out.write(b"  mov [__task_r14 + rbx*8], r14\n");
    out.write(b"  mov r12, [__task_state + rbx*8]\n");
    out.write(b"  cmp r12, 0\n");
    out.write(b"  jne .task_yield_state_ok\n");
    out.write(b"  mov qword [__task_state + rbx*8], 2\n");
    out.write(b"  mov r12, 2\n");
    out.write(b".task_yield_state_ok:\n");
    out.write(b"  cmp r12, 2\n");
    out.write(b"  jne .task_yield_no_enqueue\n");
    out.write(b"  mov qword [__task_state + rbx*8], 1\n");
    out.write(b"  mov r13, [__task_worker]\n");
    out.write(b"  mov rsi, r13\n");
    out.write(b"  shl rsi, 6\n");
    out.write(b"  add rsi, __task_w_buf\n");
    out.write(b"  mov rax, [__task_w_tail + r13*8]\n");
    out.write(b"  mov rcx, [__task_w_head + r13*8]\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  sub rdx, rcx\n");
    out.write(b"  cmp rdx, 8\n");
    out.write(b"  jae .task_yield_enqueue_global\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  and rdx, 7\n");
    out.write(b"  mov [rsi + rdx*8], rbx\n");
    out.write(b"  inc rax\n");
    out.write(b"  mov [__task_w_tail + r13*8], rax\n");
    out.write(b"  jmp .task_yield_no_enqueue\n");
    out.write(b".task_yield_enqueue_global:\n");
    out.write(b"  mov rax, [__task_g_tail]\n");
    out.write(b"  mov rcx, [__task_g_head]\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  sub rdx, rcx\n");
    out.write(b"  cmp rdx, 16\n");
    out.write(b"  jae .task_yield_no_enqueue\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  and rdx, 15\n");
    out.write(b"  mov [__task_g_buf + rdx*8], rbx\n");
    out.write(b"  inc rax\n");
    out.write(b"  mov [__task_g_tail], rax\n");
    out.write(b".task_yield_no_enqueue:\n");
    out.write(b"  mov r13, [__task_worker]\n");
    out.write(b"  mov rsi, r13\n");
    out.write(b"  shl rsi, 6\n");
    out.write(b"  add rsi, __task_w_buf\n");
    out.write(b"  mov rax, [__task_w_head + r13*8]\n");
    out.write(b"  mov rcx, [__task_w_tail + r13*8]\n");
    out.write(b"  cmp rax, rcx\n");
    out.write(b"  je .task_yield_try_global\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  and rdx, 7\n");
    out.write(b"  mov rbx, [rsi + rdx*8]\n");
    out.write(b"  inc rax\n");
    out.write(b"  mov [__task_w_head + r13*8], rax\n");
    out.write(b"  mov r10, r13\n");
    out.write(b"  jmp .task_yield_switch\n");
    out.write(b".task_yield_try_global:\n");
    out.write(b"  mov rax, [__task_g_head]\n");
    out.write(b"  mov rcx, [__task_g_tail]\n");
    out.write(b"  cmp rax, rcx\n");
    out.write(b"  je .task_yield_steal\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  and rdx, 15\n");
    out.write(b"  mov rbx, [__task_g_buf + rdx*8]\n");
    out.write(b"  inc rax\n");
    out.write(b"  mov [__task_g_head], rax\n");
    out.write(b"  mov r10, r13\n");
    out.write(b"  jmp .task_yield_switch\n");
    out.write(b".task_yield_steal:\n");
    out.write(b"  mov r11, 1\n");
    out.write(b".task_yield_steal_loop:\n");
    out.write(b"  cmp r11, 4\n");
    out.write(b"  jae .task_yield_no_ready\n");
    out.write(b"  mov r10, r13\n");
    out.write(b"  add r10, r11\n");
    out.write(b"  cmp r10, 4\n");
    out.write(b"  jb .task_yield_steal_check\n");
    out.write(b"  sub r10, 4\n");
    out.write(b".task_yield_steal_check:\n");
    out.write(b"  mov rsi, r10\n");
    out.write(b"  shl rsi, 6\n");
    out.write(b"  add rsi, __task_w_buf\n");
    out.write(b"  mov rax, [__task_w_head + r10*8]\n");
    out.write(b"  mov rcx, [__task_w_tail + r10*8]\n");
    out.write(b"  cmp rax, rcx\n");
    out.write(b"  je .task_yield_steal_next\n");
    out.write(b"  mov rdx, rax\n");
    out.write(b"  and rdx, 7\n");
    out.write(b"  mov rbx, [rsi + rdx*8]\n");
    out.write(b"  inc rax\n");
    out.write(b"  mov [__task_w_head + r10*8], rax\n");
    out.write(b"  jmp .task_yield_switch\n");
    out.write(b".task_yield_steal_next:\n");
    out.write(b"  inc r11\n");
    out.write(b"  jmp .task_yield_steal_loop\n");
    out.write(b".task_yield_no_ready:\n");
    out.write(b"  cmp r12, 2\n");
    out.write(b"  je .task_yield_no_ready_active\n");
    out.write(b"  cmp r12, 4\n");
    out.write(b"  jne .task_yield_return\n");
    // All tasks blocked and nothing runnable: program deadlock (BUG-005) —
    // deliver Deadlock (25), not the generic Unreachable (23).
    out.write(b"  mov rdi, ");
    write_u32(out, lir::trap_code_u32(lir::TrapCode::Deadlock));
    out.write(b"\n");
    out.write(b"  jmp __lang_trap\n");
    out.write(b".task_yield_no_ready_active:\n");
    out.write(b"  mov qword [__task_state + rbx*8], 2\n");
    out.write(b".task_yield_return:\n");
    out.write(b"  pop r13\n");
    out.write(b"  pop r12\n");
    out.write(b"  pop rbx\n");
    out.write(b"  ret\n");
    out.write(b".task_yield_switch:\n");
    out.write(b"  mov qword [__task_state + rbx*8], 2\n");
    out.write(b"  mov [__task_current], rbx\n");
    out.write(b"  mov [__task_worker], r10\n");
    out.write(b"  mov rsp, [__task_rsp + rbx*8]\n");
    out.write(b"  mov r15, [__task_r15 + rbx*8]\n");
    out.write(b"  mov r14, [__task_r14 + rbx*8]\n");
    out.write(b"  pop r13\n");
    out.write(b"  pop r12\n");
    out.write(b"  pop rbx\n");
    out.write(b"  ret\n");
}

fn emit_task_join_runtime(out: &mut dyn frontend::parse::Output) {
    out.write(b"\n__task_join:\n");
    out.write(b"  push rbx\n");
    out.write(b"  mov rbx, rdi\n");
    out.write(b"  cmp rbx, 16\n");
    out.write(b"  jb .task_join_loop\n");
    out.write(b"  mov rdi, ");
    write_u32(out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    out.write(b"\n");
    out.write(b"  jmp __lang_trap\n");
    out.write(b".task_join_loop:\n");
    out.write(b"  mov rax, [__task_state + rbx*8]\n");
    out.write(b"  cmp rax, 3\n");
    out.write(b"  je .task_join_done\n");
    out.write(b"  call __task_yield\n");
    out.write(b"  jmp .task_join_loop\n");
    out.write(b".task_join_done:\n");
    out.write(b"  mov qword [__task_state + rbx*8], 0\n");
    out.write(b"  pop rbx\n");
    out.write(b"  ret\n");
}

fn emit_task_sleep_runtime(out: &mut dyn frontend::parse::Output) {
    out.write(b"\n__task_sleep_ms:\n");
    out.write(b"  sub rsp, 16\n");
    out.write(b"  mov rax, rdi\n");
    out.write(b"  xor rdx, rdx\n");
    out.write(b"  mov rcx, 1000\n");
    out.write(b"  div rcx\n");
    out.write(b"  mov [rsp], rax\n");
    out.write(b"  mov rax, rdx\n");
    out.write(b"  mov rcx, 1000000\n");
    out.write(b"  imul rax, rcx\n");
    out.write(b"  mov [rsp+8], rax\n");
    out.write(b"  mov rdi, rsp\n");
    out.write(b"  xor rsi, rsi\n");
    out.write(b"  mov rax, 35\n");
    out.write(b"  syscall\n");
    out.write(b"  add rsp, 16\n");
    out.write(b"  call __task_yield\n");
    out.write(b"  ret\n");

    out.write(b"\n__task_sleep_us:\n");
    out.write(b"  sub rsp, 16\n");
    out.write(b"  mov rax, rdi\n");
    out.write(b"  xor rdx, rdx\n");
    out.write(b"  mov rcx, 1000000\n");
    out.write(b"  div rcx\n");
    out.write(b"  mov [rsp], rax\n");
    out.write(b"  mov rax, rdx\n");
    out.write(b"  mov rcx, 1000\n");
    out.write(b"  imul rax, rcx\n");
    out.write(b"  mov [rsp+8], rax\n");
    out.write(b"  mov rdi, rsp\n");
    out.write(b"  xor rsi, rsi\n");
    out.write(b"  mov rax, 35\n");
    out.write(b"  syscall\n");
    out.write(b"  add rsp, 16\n");
    out.write(b"  call __task_yield\n");
    out.write(b"  ret\n");
}

pub fn emit_task_runtime(out: &mut dyn frontend::parse::Output) {
    emit_task_spawn_runtime(out);
    emit_task_entry_exit_runtime(out);
    emit_task_yield_runtime(out);
    emit_task_join_runtime(out);
    emit_task_sleep_runtime(out);
}
