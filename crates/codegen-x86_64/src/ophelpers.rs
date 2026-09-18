use frontend::parse::Output;
use ir as lir;

pub(crate) use crate::util::{hex_digit, write_u32, write_u64_hex};

static DS_HIGH_LABEL_ID: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

pub fn write_label(out: &mut dyn Output, name: &[u8]) {
    // Emit symbol = w_<16-hex-digit-fnv1a_u64> per abi-contract §6.
    let hash = crate::util::fnv1a_u64(name);
    out.write(b"w_");
    for i in (0..64).step_by(4).rev() {
        let nib = ((hash >> i) & 0xf) as u8;
        out.write(&[hex_digit(nib)]);
    }
}

/// Emit a DS high-water update: if r15 > __lang_ds_high, update it.
/// Must be called AFTER `add r15, 8` (i.e., after the stack pointer has advanced).
pub fn emit_update_ds_high(out: &mut dyn Output) {
    let id = DS_HIGH_LABEL_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    out.write(b"  cmp r15, [__lang_ds_high]\n");
    out.write(b"  jna .ds_high_");
    write_u32(out, id);
    out.write(b"\n");
    out.write(b"  mov [__lang_ds_high], r15\n");
    out.write(b".ds_high_");
    write_u32(out, id);
    out.write(b":\n");
}

pub fn emit_push_i64(out: &mut dyn Output, v: i64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    if (i32::MIN as i64..=i32::MAX as i64).contains(&v) {
        // Fits in a sign-extended imm32: `mov r/m64, imm32` is encodable.
        out.write(b"  mov qword [r15], ");
        emit_i64(out, v);
        out.write(b"\n");
    } else {
        // `mov r/m64, imm64` has no encoding — go through a register.
        out.write(b"  mov rax, ");
        emit_i64(out, v);
        out.write(b"\n");
        out.write(b"  mov [r15], rax\n");
    }
    out.write(b"  add r15, 8\n");
    emit_update_ds_high(out);
}

pub fn emit_push_u64(out: &mut dyn Output, v: u64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    let as_i64 = v as i64;
    if as_i64 >= i32::MIN as i64 && as_i64 <= i32::MAX as i64 {
        // Fits in a sign-extended imm32: `mov r/m64, imm32` is encodable.
        out.write(b"  mov qword [r15], ");
        write_u64_hex(out, v);
        out.write(b"\n");
    } else {
        // `mov r/m64, imm64` has no encoding — go through a register.
        out.write(b"  mov rax, ");
        write_u64_hex(out, v);
        out.write(b"\n");
        out.write(b"  mov [r15], rax\n");
    }
    out.write(b"  add r15, 8\n");
    emit_update_ds_high(out);
}

pub fn emit_push_rax(out: &mut dyn Output) {
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
    emit_update_ds_high(out);
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
            // Negate in u64 space so i64::MIN (-9223372036854775808) does
            // not overflow on `-v`.
            v = v.wrapping_neg();
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
    emit_update_ds_high(out);
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
    emit_update_ds_high(out);
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
    emit_update_ds_high(out);
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
    emit_update_ds_high(out);
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

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use frontend::parse::Output;
    use std::string::String;
    use std::vec::Vec;

    struct Collect(Vec<u8>);
    impl Output for Collect {
        fn write(&mut self, bytes: &[u8]) {
            self.0.extend_from_slice(bytes);
        }
    }

    /// Every stack-*growing* data-stack push MUST emit the overflow guard
    /// (`cmp <next>, r14 ; ja __stack_overflow`, r14 = `__lang_ds_limit`)
    /// *before* writing the slot — fail-closed.
    ///
    /// Scope (important for honesty): the guard belongs only on pushes that
    /// raise the high-water mark. Net-neutral in-place ops that pop a slot and
    /// immediately push back to it (`sub r15` … `add r15`, e.g. the `not`/`neg`
    /// peephole in `word.rs` and the MMIO load helpers in `mmio.rs`) reuse an
    /// already-valid slot and correctly skip the guard — they cannot overflow.
    /// All such bare `add r15, 8` sites were audited and are paired with a
    /// preceding `sub r15, 8`; only the `emit_push_*` helpers below grow the
    /// stack, and they are the ones that carry the guard.
    ///
    /// This is the static guarantee behind STACK_OVERFLOW (claim code 10). The
    /// guard is a defensive net that is effectively unreachable from well-typed
    /// source — word calls overflow the hardware stack, not the data stack, and
    /// the stack-effect type system forbids unbounded data-stack growth (see the
    /// STACK_OVERFLOW note in `tooling-tests/tests/diag_corpus.rs`). So we verify
    /// the guard's *emission* here rather than via an impossible runtime fixture.
    #[test]
    fn data_stack_push_emits_overflow_guard() {
        let mut o = Collect(Vec::new());
        emit_push_i64(&mut o, 42);
        emit_push_u64(&mut o, 7);
        emit_push_rax(&mut o);
        let asm = String::from_utf8(o.0).unwrap();

        // One guard per push, branching to the runtime trap on overflow.
        assert_eq!(
            asm.matches("ja __stack_overflow").count(),
            3,
            "every stack-growing push helper must emit an overflow guard, got:\n{asm}"
        );
        // The bound is `__lang_ds_limit` (held in r14), compared before the write.
        assert!(
            asm.contains("cmp rax, r14"),
            "i64/u64 push must bound-check against r14:\n{asm}"
        );
        assert!(
            asm.contains("cmp rcx, r14"),
            "rax push must bound-check against r14:\n{asm}"
        );

        // Guard MUST precede the first slot write (fail-closed, not fail-open).
        let guard = asm.find("ja __stack_overflow").unwrap();
        let write = asm.find("mov qword [r15]").unwrap();
        assert!(
            guard < write,
            "overflow guard must precede the slot write:\n{asm}"
        );
    }
}
