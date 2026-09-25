use ir as lir;

use crate::ophelpers;
use crate::util::{prim_ty, prim_ty_bits_signed, type_class, type_size_bytes, write_u32};
use crate::X86_64HostedBackend;

pub enum ChannelPayloadKind {
    Primitive {
        bits: u16,
        signed: bool,
        is_bool: bool,
    },
    BoxCopy {
        bytes: u32,
    },
    Word,
}

pub fn channel_payload_kind(w: &lir::Word, ty: lir::TypeId) -> Option<ChannelPayloadKind> {
    if let Some((bits, signed)) = prim_ty_bits_signed(w, ty) {
        let is_bool = prim_ty(w, ty) == Some(lir::Prim::Bool);
        return Some(ChannelPayloadKind::Primitive {
            bits,
            signed,
            is_bool,
        });
    }
    // Slices carry a scoped descriptor (pointer + len); the channel word
    // sequence is emitted elsewhere for them, so this site returns no
    // payload kind (decision D-13 class dispatch).
    if type_class(w, ty) == lir::TypeClass::Slice {
        return None;
    }
    let size = type_size_bytes(w, ty)?;
    if size > 8 {
        return Some(ChannelPayloadKind::BoxCopy { bytes: size });
    }
    Some(ChannelPayloadKind::Word)
}

pub fn emit_chan_make(gen: &mut X86_64HostedBackend<'_>) {
    let ok = gen.fresh_label();
    let scan = gen.fresh_label();
    let found = gen.fresh_label();
    gen.out.write(b"  mov rax, [__chan_next]\n");
    gen.out.write(b"  xor rcx, rcx\n");
    gen.out.write(b".chan_make_scan_");
    write_u32(gen.out, scan);
    gen.out.write(b":\n");
    gen.out.write(b"  cmp rcx, 16\n");
    gen.out.write(b"  jb .chan_make_check_");
    write_u32(gen.out, scan);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(gen.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_make_check_");
    write_u32(gen.out, scan);
    gen.out.write(b":\n");
    gen.out.write(b"  mov rdx, [__chan_inuse + rax*8]\n");
    gen.out.write(b"  cmp rdx, 0\n");
    gen.out.write(b"  je .chan_make_found_");
    write_u32(gen.out, found);
    gen.out.write(b"\n");
    gen.out.write(b"  add rax, 1\n");
    gen.out.write(b"  cmp rax, 16\n");
    gen.out.write(b"  jb .chan_make_next_");
    write_u32(gen.out, scan);
    gen.out.write(b"\n");
    gen.out.write(b"  xor rax, rax\n");
    gen.out.write(b".chan_make_next_");
    write_u32(gen.out, scan);
    gen.out.write(b":\n");
    gen.out.write(b"  add rcx, 1\n");
    gen.out.write(b"  jmp .chan_make_scan_");
    write_u32(gen.out, scan);
    gen.out.write(b"\n");
    gen.out.write(b".chan_make_found_");
    write_u32(gen.out, found);
    gen.out.write(b":\n");
    gen.out.write(b"  mov qword [__chan_inuse + rax*8], 1\n");
    gen.out.write(b"  mov qword [__chan_head + rax*8], 0\n");
    gen.out.write(b"  mov qword [__chan_tail + rax*8], 0\n");
    gen.out.write(b"  mov rdx, rax\n");
    gen.out.write(b"  add rax, 1\n");
    gen.out.write(b"  cmp rax, 16\n");
    gen.out.write(b"  jb .chan_make_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b"\n");
    gen.out.write(b"  xor rax, rax\n");
    gen.out.write(b".chan_make_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b":\n");
    gen.out.write(b"  mov [__chan_next], rax\n");
    gen.out.write(b"  mov rax, rdx\n");
    ophelpers::emit_push_rax(gen.out, !gen.ds_guards_elided);
}

pub fn emit_chan_send(
    gen: &mut X86_64HostedBackend<'_>,
    w: &lir::Word,
    op: &lir::Op,
    sig: &lir::Sig,
) {
    let payload_ty = sig.inputs[1];
    let ok = gen.fresh_label();
    let space = gen.fresh_label();
    let live = gen.fresh_label();
    let retry = gen.fresh_label();
    let wake = gen.fresh_label();
    let gfull = gen.fresh_label();
    let wfull = gen.fresh_label();
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rdx, [r15]\n"); // payload
    match channel_payload_kind(w, payload_ty) {
        Some(ChannelPayloadKind::Primitive {
            bits,
            signed,
            is_bool,
        }) => {
            ophelpers::emit_channel_canon_prim(
                gen.out, b"rdx", b"edx", b"dl", bits, signed, is_bool,
            );
        }
        Some(ChannelPayloadKind::BoxCopy { bytes }) => {
            let ok_box = gen.fresh_label();
            ophelpers::emit_channel_box_array(gen.out, bytes, b"rdx", ok_box);
        }
        Some(ChannelPayloadKind::Word) => {}
        None => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), op.span);
            return;
        }
    }
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n"); // chan
    gen.out.write(b"  cmp rax, 16\n");
    gen.out.write(b"  jb .chan_send_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(gen.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_send_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b":\n");
    gen.out.write(b"  mov rcx, [__chan_inuse + rax*8]\n");
    gen.out.write(b"  cmp rcx, 1\n");
    gen.out.write(b"  je .chan_send_live_");
    write_u32(gen.out, live);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(gen.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_send_live_");
    write_u32(gen.out, live);
    gen.out.write(b":\n");
    gen.out.write(b".chan_send_retry_");
    write_u32(gen.out, retry);
    gen.out.write(b":\n");
    gen.out.write(b"  mov rcx, [__chan_tail + rax*8]\n");
    gen.out.write(b"  mov r8, [__chan_head + rax*8]\n");
    gen.out.write(b"  sub rcx, r8\n");
    gen.out.write(b"  cmp rcx, 64\n");
    gen.out.write(b"  jb .chan_send_space_");
    write_u32(gen.out, space);
    gen.out.write(b"\n");
    gen.out.write(b"  mov r12, rax\n");
    gen.out.write(b"  mov r13, rdx\n");
    gen.out.write(b"  mov rdx, [__task_current]\n");
    gen.out
        .write(b"  mov r8, [__chan_wait_send_tail + r12*8]\n");
    gen.out
        .write(b"  mov r9, [__chan_wait_send_head + r12*8]\n");
    gen.out.write(b"  mov r10, r8\n");
    gen.out.write(b"  sub r10, r9\n");
    gen.out.write(b"  cmp r10, 8\n");
    gen.out.write(b"  jb .chan_send_wait_space_");
    write_u32(gen.out, wfull);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(gen.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_send_wait_space_");
    write_u32(gen.out, wfull);
    gen.out.write(b":\n");
    gen.out.write(b"  mov r10, r8\n");
    gen.out.write(b"  and r10, 7\n");
    gen.out.write(b"  mov r9, r12\n");
    gen.out.write(b"  shl r9, 3\n");
    gen.out.write(b"  add r9, r10\n");
    gen.out.write(b"  mov [__chan_wait_send_buf + r9*8], rdx\n");
    gen.out.write(b"  add r8, 1\n");
    gen.out
        .write(b"  mov [__chan_wait_send_tail + r12*8], r8\n");
    gen.out.write(b"  mov qword [__task_state + rdx*8], 4\n");
    gen.out.write(b"  call __task_yield\n");
    gen.out.write(b"  mov rax, r12\n");
    gen.out.write(b"  mov rdx, r13\n");
    gen.out.write(b"  jmp .chan_send_retry_");
    write_u32(gen.out, retry);
    gen.out.write(b"\n");
    gen.out.write(b".chan_send_space_");
    write_u32(gen.out, space);
    gen.out.write(b":\n");
    gen.out.write(b"  mov rcx, [__chan_tail + rax*8]\n");
    gen.out.write(b"  mov r8, rcx\n");
    gen.out.write(b"  and r8, 63\n");
    gen.out.write(b"  mov r9, rax\n");
    gen.out.write(b"  shl r9, 6\n");
    gen.out.write(b"  add r9, r8\n");
    gen.out.write(b"  mov [__chan_buf + r9*8], rdx\n");
    gen.out.write(b"  add rcx, 1\n");
    gen.out.write(b"  mov [__chan_tail + rax*8], rcx\n");
    gen.out
        .write(b"  mov r8, [__chan_wait_recv_head + rax*8]\n");
    gen.out
        .write(b"  mov r9, [__chan_wait_recv_tail + rax*8]\n");
    gen.out.write(b"  cmp r8, r9\n");
    gen.out.write(b"  je .chan_send_wake_done_");
    write_u32(gen.out, wake);
    gen.out.write(b"\n");
    gen.out.write(b"  mov r10, r8\n");
    gen.out.write(b"  and r10, 7\n");
    gen.out.write(b"  mov r11, rax\n");
    gen.out.write(b"  shl r11, 3\n");
    gen.out.write(b"  add r11, r10\n");
    gen.out
        .write(b"  mov r10, [__chan_wait_recv_buf + r11*8]\n");
    gen.out.write(b"  add r8, 1\n");
    gen.out
        .write(b"  mov [__chan_wait_recv_head + rax*8], r8\n");
    gen.out.write(b"  mov qword [__task_state + r10*8], 1\n");
    gen.out.write(b"  mov r8, [__task_g_tail]\n");
    gen.out.write(b"  mov r9, [__task_g_head]\n");
    gen.out.write(b"  mov r11, r8\n");
    gen.out.write(b"  sub r11, r9\n");
    gen.out.write(b"  cmp r11, 16\n");
    gen.out.write(b"  jb .chan_send_wake_space_");
    write_u32(gen.out, gfull);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(
        gen.out,
        lir::trap_code_u32(lir::TrapCode::TaskQueueOverflow),
    );
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_send_wake_space_");
    write_u32(gen.out, gfull);
    gen.out.write(b":\n");
    gen.out.write(b"  mov r11, r8\n");
    gen.out.write(b"  and r11, 15\n");
    gen.out.write(b"  mov [__task_g_buf + r11*8], r10\n");
    gen.out.write(b"  add r8, 1\n");
    gen.out.write(b"  mov [__task_g_tail], r8\n");
    gen.out.write(b".chan_send_wake_done_");
    write_u32(gen.out, wake);
    gen.out.write(b":\n");
}

pub fn emit_chan_recv(
    gen: &mut X86_64HostedBackend<'_>,
    w: &lir::Word,
    op: &lir::Op,
    sig: &lir::Sig,
) {
    let out_ty = sig.outputs[0];
    let ok = gen.fresh_label();
    let has = gen.fresh_label();
    let live = gen.fresh_label();
    let retry = gen.fresh_label();
    let wake = gen.fresh_label();
    let gfull = gen.fresh_label();
    let wfull = gen.fresh_label();
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov r11, [r15]\n"); // chan
    gen.out.write(b"  cmp r11, 16\n");
    gen.out.write(b"  jb .chan_recv_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(gen.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_recv_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b":\n");
    gen.out.write(b"  mov rcx, [__chan_inuse + r11*8]\n");
    gen.out.write(b"  cmp rcx, 1\n");
    gen.out.write(b"  je .chan_recv_live_");
    write_u32(gen.out, live);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(gen.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_recv_live_");
    write_u32(gen.out, live);
    gen.out.write(b":\n");
    gen.out.write(b".chan_recv_retry_");
    write_u32(gen.out, retry);
    gen.out.write(b":\n");
    gen.out.write(b"  mov rcx, [__chan_head + r11*8]\n");
    gen.out.write(b"  mov r8, [__chan_tail + r11*8]\n");
    gen.out.write(b"  cmp rcx, r8\n");
    gen.out.write(b"  jne .chan_recv_has_");
    write_u32(gen.out, has);
    gen.out.write(b"\n");
    gen.out.write(b"  mov r12, r11\n");
    gen.out.write(b"  mov rdx, [__task_current]\n");
    gen.out
        .write(b"  mov r8, [__chan_wait_recv_tail + r11*8]\n");
    gen.out
        .write(b"  mov r9, [__chan_wait_recv_head + r11*8]\n");
    gen.out.write(b"  mov r10, r8\n");
    gen.out.write(b"  sub r10, r9\n");
    gen.out.write(b"  cmp r10, 8\n");
    gen.out.write(b"  jb .chan_recv_wait_space_");
    write_u32(gen.out, wfull);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(gen.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_recv_wait_space_");
    write_u32(gen.out, wfull);
    gen.out.write(b":\n");
    gen.out.write(b"  mov r10, r8\n");
    gen.out.write(b"  and r10, 7\n");
    gen.out.write(b"  mov r9, r11\n");
    gen.out.write(b"  shl r9, 3\n");
    gen.out.write(b"  add r9, r10\n");
    gen.out.write(b"  mov [__chan_wait_recv_buf + r9*8], rdx\n");
    gen.out.write(b"  add r8, 1\n");
    gen.out
        .write(b"  mov [__chan_wait_recv_tail + r11*8], r8\n");
    gen.out.write(b"  mov qword [__task_state + rdx*8], 4\n");
    gen.out.write(b"  call __task_yield\n");
    gen.out.write(b"  mov r11, r12\n");
    gen.out.write(b"  jmp .chan_recv_retry_");
    write_u32(gen.out, retry);
    gen.out.write(b"\n");
    gen.out.write(b".chan_recv_has_");
    write_u32(gen.out, has);
    gen.out.write(b":\n");
    gen.out.write(b"  mov r9, rcx\n");
    gen.out.write(b"  and r9, 63\n");
    gen.out.write(b"  mov r10, r11\n");
    gen.out.write(b"  shl r10, 6\n");
    gen.out.write(b"  add r10, r9\n");
    gen.out.write(b"  mov rax, [__chan_buf + r10*8]\n");
    match channel_payload_kind(w, out_ty) {
        Some(ChannelPayloadKind::Primitive {
            bits,
            signed,
            is_bool,
        }) => {
            ophelpers::emit_channel_canon_prim(
                gen.out, b"rax", b"eax", b"al", bits, signed, is_bool,
            );
        }
        Some(ChannelPayloadKind::BoxCopy { .. }) => {}
        Some(ChannelPayloadKind::Word) => {}
        None => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), op.span);
            return;
        }
    }
    gen.out.write(b"  add rcx, 1\n");
    gen.out.write(b"  mov [__chan_head + r11*8], rcx\n");
    gen.out
        .write(b"  mov r8, [__chan_wait_send_head + r11*8]\n");
    gen.out
        .write(b"  mov r9, [__chan_wait_send_tail + r11*8]\n");
    gen.out.write(b"  cmp r8, r9\n");
    gen.out.write(b"  je .chan_recv_wake_done_");
    write_u32(gen.out, wake);
    gen.out.write(b"\n");
    gen.out.write(b"  mov r10, r8\n");
    gen.out.write(b"  and r10, 7\n");
    gen.out.write(b"  mov rdx, r11\n");
    gen.out.write(b"  shl rdx, 3\n");
    gen.out.write(b"  add rdx, r10\n");
    gen.out
        .write(b"  mov r10, [__chan_wait_send_buf + rdx*8]\n");
    gen.out.write(b"  add r8, 1\n");
    gen.out
        .write(b"  mov [__chan_wait_send_head + r11*8], r8\n");
    gen.out.write(b"  mov qword [__task_state + r10*8], 1\n");
    gen.out.write(b"  mov r8, [__task_g_tail]\n");
    gen.out.write(b"  mov r9, [__task_g_head]\n");
    gen.out.write(b"  mov rdx, r8\n");
    gen.out.write(b"  sub rdx, r9\n");
    gen.out.write(b"  cmp rdx, 16\n");
    gen.out.write(b"  jb .chan_recv_wake_space_");
    write_u32(gen.out, gfull);
    gen.out.write(b"\n");
    gen.out.write(b"  mov rdi, ");
    write_u32(
        gen.out,
        lir::trap_code_u32(lir::TrapCode::TaskQueueOverflow),
    );
    gen.out.write(b"\n");
    gen.out.write(b"  jmp __lang_trap\n");
    gen.out.write(b".chan_recv_wake_space_");
    write_u32(gen.out, gfull);
    gen.out.write(b":\n");
    gen.out.write(b"  mov rdx, r8\n");
    gen.out.write(b"  and rdx, 15\n");
    gen.out.write(b"  mov [__task_g_buf + rdx*8], r10\n");
    gen.out.write(b"  add r8, 1\n");
    gen.out.write(b"  mov [__task_g_tail], r8\n");
    gen.out.write(b".chan_recv_wake_done_");
    write_u32(gen.out, wake);
    gen.out.write(b":\n");
    ophelpers::emit_push_rax(gen.out, !gen.ds_guards_elided);
}
