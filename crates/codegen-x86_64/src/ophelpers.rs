use frontend::parse::Output;
use ir as lir;

pub(crate) use crate::util::{hex_digit, write_u32, write_u64_hex};

pub fn write_label(out: &mut dyn Output, name: &[u8]) {
    out.write(b"w_");
    for &b in name {
        let hi = b >> 4;
        let lo = b & 0xf;
        out.write(&[hex_digit(hi), hex_digit(lo)]);
    }
}

pub fn emit_push_i64(out: &mut dyn Output, v: i64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov qword [r15], ");
    emit_i64(out, v);
    out.write(b"\n");
    out.write(b"  add r15, 8\n");
}

pub fn emit_push_u64(out: &mut dyn Output, v: u64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov qword [r15], ");
    write_u64_hex(out, v);
    out.write(b"\n");
    out.write(b"  add r15, 8\n");
}

pub fn emit_push_rax(out: &mut dyn Output) {
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

pub fn emit_i64(out: &mut dyn Output, mut v: i64) {
    let mut buf = [0u8; 24];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        if v < 0 {
            out.write(b"-");
            v = -v;
        }
        let mut u = v as u64;
        while u > 0 && n < buf.len() {
            buf[n] = b'0' + (u % 10) as u8;
            n += 1;
            u /= 10;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

pub fn emit_dup(out: &mut dyn Output) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

pub fn emit_drop(out: &mut dyn Output) {
    out.write(b"  sub r15, 8\n");
}

pub fn emit_swap(out: &mut dyn Output) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  mov rcx, [r15-16]\n");
    out.write(b"  mov [r15-8], rcx\n");
    out.write(b"  mov [r15-16], rax\n");
}

pub fn emit_binop(out: &mut dyn Output, op: &[u8]) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rcx, [r15]\n");
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  ");
    out.write(op);
    out.write(b" rax, rcx\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

pub fn emit_cmp(out: &mut dyn Output, setcc: &[u8]) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rcx, [r15]\n");
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  cmp rax, rcx\n");
    out.write(b"  ");
    out.write(setcc);
    out.write(b" al\n");
    out.write(b"  movzx rax, al\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

pub fn emit_store_local(out: &mut dyn Output, idx: u32) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  mov [rsp+");
    write_u32(out, idx * 8);
    out.write(b"], rax\n");
}

pub fn emit_load_local(out: &mut dyn Output, idx: u32) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov rax, [rsp+");
    write_u32(out, idx * 8);
    out.write(b"]\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

pub fn emit_stack_overflow(out: &mut dyn Output) {
    out.write(b"__stack_overflow:\n");
    out.write(b"  mov rdi, ");
    write_u32(out, lir::trap_code_u32(lir::TrapCode::StackOverflow));
    out.write(b"\n");
    out.write(b"  jmp __lang_trap\n");
}

pub fn emit_channel_canon_prim(
    out: &mut dyn Output,
    reg: &[u8],
    reg32: &[u8],
    reg8: &[u8],
    bits: u16,
    signed: bool,
    is_bool: bool,
) {
    if bits < 64 {
        if bits <= 32 {
            out.write(b"  and ");
            out.write(reg32);
            out.write(b", ");
            write_u64_hex(out, mask_for_bits(bits));
            out.write(b"\n");
        } else {
            out.write(b"  and ");
            out.write(reg);
            out.write(b", ");
            write_u64_hex(out, mask_for_bits(bits));
            out.write(b"\n");
        }
        if signed {
            let sh = 64u32 - (bits as u32);
            out.write(b"  shl ");
            out.write(reg);
            out.write(b", ");
            write_u32(out, sh);
            out.write(b"\n");
            out.write(b"  sar ");
            out.write(reg);
            out.write(b", ");
            write_u32(out, sh);
            out.write(b"\n");
        }
    }

    if is_bool {
        out.write(b"  cmp ");
        out.write(reg);
        out.write(b", 0\n");
        out.write(b"  setne ");
        out.write(reg8);
        out.write(b"\n");
        out.write(b"  movzx ");
        out.write(reg);
        out.write(b", ");
        out.write(reg8);
        out.write(b"\n");
    }
}

use crate::util::mask_for_bits;

pub fn emit_channel_box_array(out: &mut dyn Output, bytes: u32, src_reg: &[u8], ok: u32) {
    out.write(b"  mov r12, ");
    out.write(src_reg);
    out.write(b"\n");
    out.write(b"  xor rdi, rdi\n");
    out.write(b"  mov rsi, ");
    write_u32(out, bytes);
    out.write(b"\n");
    out.write(b"  mov rdx, 3\n");
    out.write(b"  mov r10, 0x22\n");
    out.write(b"  mov r8, -1\n");
    out.write(b"  xor r9, r9\n");
    out.write(b"  mov rax, 9\n");
    out.write(b"  syscall\n");
    out.write(b"  cmp rax, 0\n");
    out.write(b"  jns .chan_box_ok_");
    write_u32(out, ok);
    out.write(b"\n");
    out.write(b"  mov rdi, 23\n");
    out.write(b"  jmp __lang_trap\n");
    out.write(b".chan_box_ok_");
    write_u32(out, ok);
    out.write(b":\n");
    out.write(b"  mov rdi, rax\n");
    out.write(b"  mov rsi, r12\n");
    out.write(b"  mov rcx, ");
    write_u32(out, bytes);
    out.write(b"\n");
    out.write(b"  rep movsb\n");
    out.write(b"  mov ");
    out.write(src_reg);
    out.write(b", rdi\n");
}
