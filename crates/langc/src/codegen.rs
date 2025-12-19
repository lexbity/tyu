use frontend::{
    fixed::FixedVec,
    parse::{ModuleAst, Output},
    span::Span,
};
use ir as lir;
use semantics::types::WordSig;
use semantics::typecheck;
use crate::util::{slice_span, line_col, write_u32, write_u64_hex};
use crate::iface::find_word_decl;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsmMode {
    Executable,
    Object,
}

pub struct IrAsmGen<'a> {
    pub module: &'a ModuleAst,
    pub src: &'a [u8],
    pub out: &'a mut dyn Output,
    pub mode: AsmMode,
    pub label_id: u32,
    pub uses_channels: bool,
    pub uses_mmio: bool,
    pub str_len: usize,
    pub str_spans: [Span; 128],
    pub str_ids: [u32; 128],
    pub debug_trap_loc: bool,
    pub cur_word_id: u32,
}

impl<'a> IrAsmGen<'a> {
    pub fn new(module: &'a ModuleAst, src: &'a [u8], out: &'a mut dyn Output, debug_trap_loc: bool, mode: AsmMode) -> Self {
        Self {
            module,
            src,
            out,
            mode,
            label_id: 0,
            uses_channels: false,
            uses_mmio: false,
            str_len: 0,
            str_spans: [Span::new(0, 0); 128],
            str_ids: [0u32; 128],
            debug_trap_loc,
            cur_word_id: 0,
        }
    }

    pub fn fresh_label(&mut self) -> u32 {
        let id = self.label_id;
        self.label_id = self.label_id.wrapping_add(1);
        id
    }

    pub fn emit_prelude(&mut self) -> Result<(), u32> {
        match self.mode {
            AsmMode::Executable => {
                self.out.write(b"format ELF64 executable\n");
                self.out.write(b"entry __lang_start\n\n");

                self.out.write(b"segment readable executable\n");
                self.out.write(b"__lang_start:\n");
                self.out.write(b"  mov r15, __lang_ds_base\n");
                self.out.write(b"  mov r14, __lang_ds_limit\n");

                let main_decl = find_word_decl(self.module, self.src, b"main").ok_or(7001u32)?;
                let main_sig = main_decl
                    .sig
                    .map(|s| typecheck::parse_word_sig(self.src, s).ok())
                    .flatten()
                    .unwrap_or(WordSig::empty());

                self.out.write(b"  call ");
                write_label(self.out, b"main");
                self.out.write(b"\n");

                if main_sig.out_len > 0 {
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rdi, [r15]\n");
                    self.out.write(b"  and rdi, 0xff\n");
                } else {
                    self.out.write(b"  xor rdi, rdi\n");
                }
                self.out.write(b"  mov rax, 60\n");
                self.out.write(b"  syscall\n\n");

                self.out.write(b"__lang_trap:\n");
                self.out.write(b"  mov rax, 60\n");
                self.out.write(b"  syscall\n\n");
                if self.debug_trap_loc {
                    self.out.write(b"__lang_trap_loc:\n");
                    // Signature: (code:u32, file_id:u32, line:u32, word_id:u32) -> ! 
                    // Hosted stub: ignore location and exit with `code`.
                    self.out.write(b"  mov rax, 60\n");
                    self.out.write(b"  syscall\n\n");
                }
                emit_stack_overflow(self.out);
                Ok(())
            }
            AsmMode::Object => {
                self.out.write(b"format ELF64\n\n");
                self.out.write(b"section '.text' executable\n");
                // Runtime-provided symbols.
                self.out.write(b"extrn __lang_trap\n");
                self.out.write(b"extrn __lang_trap_loc\n");
                self.out.write(b"extrn __stack_overflow\n");
                self.out.write(b"extrn __mmio_mem\n");
                self.out.write(b"extrn __chan_next\n");
                self.out.write(b"extrn __chan_head\n");
                self.out.write(b"extrn __chan_tail\n");
                self.out.write(b"extrn __chan_buf\n");
                self.out.write(b"\n");
                Ok(())
            }
        }
    }

    pub fn emit_postlude(&mut self) -> Result<(), u32> {
        match self.mode {
            AsmMode::Executable => {
                // Emit string literals (as `str` structs pointing to bytes) into the executable segment.
                for i in 0..self.str_len {
                    let id = self.str_ids[i];
                    let span = self.str_spans[i];
                    let bytes = decode_string_bytes(self.src, span).ok_or(7120u32)?;

                    self.out.write(b"\n__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b":\n");
                    self.out.write(b"  dq __lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes\n");
                    self.out.write(b"  dq ");
                    write_u32(self.out, bytes.len() as u32);
                    self.out.write(b"\n");

                    self.out.write(b"__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes db ");
                    for (j, b) in bytes.iter().enumerate() {
                        if j != 0 {
                            self.out.write(b",");
                        }
                        write_u32(self.out, *b as u32);
                    }
                    if bytes.len() == 0 {
                        self.out.write(b"0");
                    }
                    self.out.write(b"\n");
                }

                self.out.write(b"\nsegment readable writeable\n");
                if self.uses_channels {
                    // Hosted channels (Milestone 16): fixed-size per-channel ring buffers.
                    self.out.write(b"__chan_next dq 0\n");
                    self.out.write(b"__chan_head rq 16\n");
                    self.out.write(b"__chan_tail rq 16\n");
                    self.out.write(b"__chan_buf rq ");
                    write_u32(self.out, 16 * 64);
                    self.out.write(b"\n");
                }
                if self.uses_mmio {
                    // Hosted MMIO (Milestone 6): fixed-size simulated MMIO region.
                    self.out.write(b"__mmio_mem rb 65536\n");
                }
                self.out.write(b"__lang_ds_base rb 65536\n");
                self.out.write(b"__lang_ds_limit:\n");
                Ok(())
            }
            AsmMode::Object => {
                if self.str_len == 0 {
                    return Ok(());
                }
                self.out.write(b"section '.data' writeable\n");
                for i in 0..self.str_len {
                    let id = self.str_ids[i];
                    let span = self.str_spans[i];
                    let bytes = decode_string_bytes(self.src, span).ok_or(7120u32)?;

                    self.out.write(b"\n__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b":\n");
                    self.out.write(b"  dq __lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes\n");
                    self.out.write(b"  dq ");
                    write_u32(self.out, bytes.len() as u32);
                    self.out.write(b"\n");

                    self.out.write(b"__lang_str_");
                    write_u32(self.out, id);
                    self.out.write(b"_bytes db ");
                    for (j, b) in bytes.iter().enumerate() {
                        if j != 0 {
                            self.out.write(b",");
                        }
                        write_u32(self.out, *b as u32);
                    }
                    if bytes.len() == 0 {
                        self.out.write(b"0");
                    }
                    self.out.write(b"\n");
                }
                Ok(())
            }
        }
    }

    fn intern_str(&mut self, span: Span) -> Result<u32, u32> {
        for i in 0..self.str_len {
            if slice_span(self.src, self.str_spans[i]) == slice_span(self.src, span) {
                return Ok(self.str_ids[i]);
            }
        }
        if self.str_len >= self.str_spans.len() {
            return Err(7121);
        }
        let id = self.fresh_label();
        self.str_spans[self.str_len] = span;
        self.str_ids[self.str_len] = id;
        self.str_len += 1;
        Ok(id)
    }

    pub fn emit_word(&mut self, w: &lir::Word) -> Result<(), u32> {
        self.cur_word_id = fnv1a_u32(w.name.as_bytes());
        self.out.write(b"\n");
        if self.mode == AsmMode::Object {
            self.out.write(b"public ");
            write_label(self.out, w.name.as_bytes());
            self.out.write(b"\n");
        }
        write_label(self.out, w.name.as_bytes());
        self.out.write(b":\n");

        let base = self.fresh_label();

        let slots = max_local_slot_ir(w).map(|m| (m as u32) + 1).unwrap_or(0);
        let frame_bytes = locals_bytes_ir(slots);
        if frame_bytes > 0 {
            self.out.write(b"  sub rsp, ");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }

        // Jump to entry so we don't depend on block emission order.
        self.out.write(b"  jmp .b");
        write_u32(self.out, base);
        self.out.write(b"_");
        write_u32(self.out, w.entry.0 as u32);
        self.out.write(b"\n");

        for b in w.blocks.iter() {
            self.out.write(b".b");
            write_u32(self.out, base);
            self.out.write(b"_");
            write_u32(self.out, b.id.0 as u32);
            self.out.write(b":\n");
            for op in b.ops.iter() {
                self.emit_op(w, op, base)?;
            }
        }

        self.out.write(b".endword_");
        write_u32(self.out, base);
        self.out.write(b":\n");
        if frame_bytes > 0 {
            self.out.write(b"  add rsp, ");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }
        self.out.write(b"  ret\n");
        Ok(())
    }

    fn emit_op(&mut self, w: &lir::Word, op: &lir::Op, base: u32) -> Result<(), u32> {
        match op.kind {
            lir::OpKind::ConstI64(v) => {
                emit_push_i64(self.out, v);
                Ok(())
            }
            lir::OpKind::ConstBool(v) => {
                emit_push_i64(self.out, if v { 1 } else { 0 });
                Ok(())
            }
            lir::OpKind::ConstStr(span) => {
                let id = self.intern_str(span)?;
                self.out.write(b"  mov rax, __lang_str_");
                write_u32(self.out, id);
                self.out.write(b"\n");
                emit_push_rax(self.out);
                Ok(())
            }

            lir::OpKind::AddrOf { const_addr: Some(addr), .. } => {
                self.uses_mmio = true;
                emit_push_u64(self.out, addr);
                Ok(())
            }
            lir::OpKind::AddrOf { const_addr: None, .. } => Err(7101),
            lir::OpKind::MmioPlace { addr, .. } => {
                self.uses_mmio = true;
                emit_push_u64(self.out, addr);
                Ok(())
            }

            lir::OpKind::ScopedEnter { .. } => Ok(()),

            lir::OpKind::Dup { .. } => {
                emit_dup(self.out);
                Ok(())
            }
            lir::OpKind::Drop { .. } => {
                emit_drop(self.out);
                Ok(())
            }
            lir::OpKind::Swap { .. } => {
                emit_swap(self.out);
                Ok(())
            }

            lir::OpKind::AddI64 => {
                emit_binop(self.out, b"add");
                Ok(())
            }
            lir::OpKind::SubI64 => {
                emit_binop(self.out, b"sub");
                Ok(())
            }
            lir::OpKind::MulI64 => {
                emit_binop(self.out, b"imul");
                Ok(())
            }
            lir::OpKind::Cmp { kind, .. } => {
                let setcc: &[u8] = match kind {
                    lir::CmpKind::Lt => b"setl",
                    lir::CmpKind::Le => b"setle",
                    lir::CmpKind::Gt => b"setg",
                    lir::CmpKind::Ge => b"setge",
                    lir::CmpKind::Eq => b"sete",
                    lir::CmpKind::Ne => b"setne",
                };
                emit_cmp(self.out, setcc);
                Ok(())
            }
            lir::OpKind::AndBool => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rcx, [r15]\n");
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  and rax, rcx\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  setne al\n");
                self.out.write(b"  movzx rax, al\n");
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(())
            }
            lir::OpKind::OrBool => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rcx, [r15]\n");
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  or rax, rcx\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  setne al\n");
                self.out.write(b"  movzx rax, al\n");
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(())
            }
            lir::OpKind::NotBool => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  sete al\n");
                self.out.write(b"  movzx rax, al\n");
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(())
            }

            lir::OpKind::LocalSet { slot, .. } => {
                emit_store_local(self.out, slot as u32);
                Ok(())
            }
            lir::OpKind::LocalGet { slot, .. } => {
                emit_load_local(self.out, slot as u32);
                Ok(())
            }

            lir::OpKind::Cast { from, to } => {
                self.emit_cast(w, from, to);
                Ok(())
            }
            lir::OpKind::Bitcast { .. } => Ok(()),

            lir::OpKind::Call { name, .. } => {
                let n = name.as_bytes();
                if n == b"platform.io.log" {
                    // Stack: `( str -- )` where `str` is `*const { ptr:u64, len:u64 }`.
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rcx, [r15]\n");
                    self.out.write(b"  mov rsi, [rcx]\n");
                    self.out.write(b"  mov rdx, [rcx+8]\n");
                    self.out.write(b"  mov rdi, 2\n");
                    self.out.write(b"  mov rax, 1\n");
                    self.out.write(b"  syscall\n");
                    return Ok(())
                }
                if n == b"platform.channel.make" {
                    // Stack: `( -- Chan(T) )` (hosted: returns small integer handle).
                    self.uses_channels = true;
                    let ok = self.fresh_label();
                    self.out.write(b"  mov rax, [__chan_next]\n");
                    self.out.write(b"  cmp rax, 16\n");
                    self.out.write(b"  jb .chan_make_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_make_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, rax\n");
                    self.out.write(b"  add rax, 1\n");
                    self.out.write(b"  mov [__chan_next], rax\n");
                    self.out.write(b"  mov rax, rcx\n");
                    emit_push_rax(self.out);
                    return Ok(())
                }
                if n == b"platform.channel.send" {
                    // Stack: `( Chan(T) T -- )` (hosted: enqueue 64-bit payload).
                    self.uses_channels = true;
                    let ok = self.fresh_label();
                    let space = self.fresh_label();
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rdx, [r15]\n"); // payload
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rax, [r15]\n"); // chan
                    self.out.write(b"  cmp rax, 16\n");
                    self.out.write(b"  jb .chan_send_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_send_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, [__chan_tail + rax*8]\n");
                    self.out.write(b"  mov r8, [__chan_head + rax*8]\n");
                    self.out.write(b"  sub rcx, r8\n");
                    self.out.write(b"  cmp rcx, 64\n");
                    self.out.write(b"  jb .chan_send_space_");
                    write_u32(self.out, space);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_send_space_");
                    write_u32(self.out, space);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, [__chan_tail + rax*8]\n");
                    self.out.write(b"  mov r8, rcx\n");
                    self.out.write(b"  and r8, 63\n");
                    self.out.write(b"  mov r9, rax\n");
                    self.out.write(b"  shl r9, 6\n");
                    self.out.write(b"  add r9, r8\n");
                    self.out.write(b"  mov [__chan_buf + r9*8], rdx\n");
                    self.out.write(b"  add rcx, 1\n");
                    self.out.write(b"  mov [__chan_tail + rax*8], rcx\n");
                    return Ok(())
                }
                if n == b"platform.channel.recv" {
                    // Stack: `( Chan(T) -- T )` (hosted: dequeue 64-bit payload).
                    self.uses_channels = true;
                    let ok = self.fresh_label();
                    let has = self.fresh_label();
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov r11, [r15]\n"); // chan
                    self.out.write(b"  cmp r11, 16\n");
                    self.out.write(b"  jb .chan_recv_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_recv_ok_");
                    write_u32(self.out, ok);
                    self.out.write(b":\n");
                    self.out.write(b"  mov rcx, [__chan_head + r11*8]\n");
                    self.out.write(b"  mov r8, [__chan_tail + r11*8]\n");
                    self.out.write(b"  cmp rcx, r8\n");
                    self.out.write(b"  jne .chan_recv_has_");
                    write_u32(self.out, has);
                    self.out.write(b"\n");
                    self.out.write(b"  mov rdi, ");
                    write_u32(self.out, lir::trap_code_u32(lir::TrapCode::Unreachable));
                    self.out.write(b"\n");
                    self.out.write(b"  jmp __lang_trap\n");
                    self.out.write(b".chan_recv_has_");
                    write_u32(self.out, has);
                    self.out.write(b":\n");
                    self.out.write(b"  mov r9, rcx\n");
                    self.out.write(b"  and r9, 63\n");
                    self.out.write(b"  mov r10, r11\n");
                    self.out.write(b"  shl r10, 6\n");
                    self.out.write(b"  add r10, r9\n");
                    self.out.write(b"  mov rax, [__chan_buf + r10*8]\n");
                    self.out.write(b"  add rcx, 1\n");
                    self.out.write(b"  mov [__chan_head + r11*8], rcx\n");
                    emit_push_rax(self.out);
                    return Ok(())
                }
                if n == b"platform.time.now_ms" {
                    // Stack: `( -- i64 )`. Uses `clock_gettime(CLOCK_MONOTONIC, &ts)`.
                    self.out.write(b"  sub rsp, 16\n");
                    self.out.write(b"  mov rdi, 1\n");
                    self.out.write(b"  mov rsi, rsp\n");
                    self.out.write(b"  mov rax, 228\n");
                    self.out.write(b"  syscall\n");
                    self.out.write(b"  mov rax, [rsp]\n");
                    self.out.write(b"  imul rax, 1000\n");
                    self.out.write(b"  mov r9, rax\n");
                    self.out.write(b"  mov rax, [rsp+8]\n");
                    self.out.write(b"  xor rdx, rdx\n");
                    self.out.write(b"  mov rcx, 1000000\n");
                    self.out.write(b"  div rcx\n");
                    self.out.write(b"  add rax, r9\n");
                    self.out.write(b"  add rsp, 16\n");
                    emit_push_rax(self.out);
                    return Ok(())
                }
                if n == b"platform.task.yield" {
                    // Hosted baseline: `sched_yield` syscall (Linux x86_64: 24).
                    self.out.write(b"  mov rax, 24\n");
                    self.out.write(b"  syscall\n");
                    return Ok(())
                }
                if n == b"platform.critical.enter" || n == b"platform.critical.exit" {
                    // Single-thread hosted baseline: no-op.
                    return Ok(())
                }
                self.out.write(b"  call ");
                write_label(self.out, name.as_bytes());
                self.out.write(b"\n");
                Ok(())
            }

            lir::OpKind::Load { .. } => Err(7103),
            lir::OpKind::Store { .. } => Err(7104),

            lir::OpKind::MmioVolLoad { ty, .. } => {
                self.uses_mmio = true;
                let (bits, signed) = prim_ty_bits_signed(w, ty).ok_or(7105u32)?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                emit_mmio_load(self, width, signed, op.span);
                Ok(())
            }
            lir::OpKind::MmioVolStore { ty, .. } => {
                self.uses_mmio = true;
                let (bits, _signed) = prim_ty_bits_signed(w, ty).ok_or(7106u32)?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                emit_mmio_store(self, width, op.span);
                Ok(())
            }
            lir::OpKind::MmioVolLoadField { reg_ty, field_ty, mask, shift, .. } => {
                self.uses_mmio = true;
                let (reg_bits, _reg_signed) = prim_ty_bits_signed(w, reg_ty).ok_or(7107u32)?;
                let reg_width = core::cmp::max(1u32, (reg_bits as u32) / 8);
                let (field_bits, field_signed) = prim_ty_bits_signed(w, field_ty).ok_or(7107u32)?;
                emit_mmio_load_field(self, reg_width, field_bits, field_signed, mask, shift, op.span);
                Ok(())
            }
            lir::OpKind::MmioVolStoreField { reg_ty, mask, shift, .. } => {
                self.uses_mmio = true;
                let (reg_bits, _reg_signed) = prim_ty_bits_signed(w, reg_ty).ok_or(7108u32)?;
                let reg_width = core::cmp::max(1u32, (reg_bits as u32) / 8);
                emit_mmio_store_field(self, reg_width, mask, shift, op.span);
                Ok(())
            }

            lir::OpKind::CheckSubtype { .. } => {
                Err(7110)
            }
            lir::OpKind::TrapIfFalse { code } => {
                let ok = self.fresh_label();
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  jne .trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b"\n");
                self.emit_trap_with_loc(lir::trap_code_u32(code), op.span);
                self.out.write(b".trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b":\n");
                let _ = base;
                Ok(())
            }

            lir::OpKind::Br { target } => {
                self.out.write(b"  jmp .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, target.0 as u32);
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::BrIf { then_tgt, else_tgt } => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  je .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, else_tgt.0 as u32);
                self.out.write(b"\n");
                self.out.write(b"  jmp .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, then_tgt.0 as u32);
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::Ret => {
                self.out.write(b"  jmp .endword_");
                write_u32(self.out, base);
                self.out.write(b"\n");
                Ok(())
            }
        }
    }

    fn emit_cast(&mut self, w: &lir::Word, from: lir::TypeId, to: lir::TypeId) {
        let from_ty = w.types.get(from.0 as usize).map(|a| a.as_bytes()).unwrap_or(b"");
        let to_ty = w.types.get(to.0 as usize).map(|a| a.as_bytes()).unwrap_or(b"");
        if from_ty == to_ty {
            return;
        }

        let Some((from_bits, from_signed)) = prim_bits_signed(from_ty) else {
            return;
        };
        let Some((to_bits, to_signed)) = prim_bits_signed(to_ty) else {
            return;
        };

        // Top value lives at [r15-8].
        self.out.write(b"  mov rax, [r15-8]\n");

        // Canonicalize to the source type (truncate + sign/zero extend).
        if from_bits < 64 {
            if from_bits <= 32 {
                self.out.write(b"  and eax, ");
                write_u64_hex(self.out, mask_for_bits(from_bits));
                self.out.write(b"\n");
            } else {
                self.out.write(b"  and rax, ");
                write_u64_hex(self.out, mask_for_bits(from_bits));
                self.out.write(b"\n");
            }
            if from_signed {
                let sh = 64u32 - (from_bits as u32);
                self.out.write(b"  shl rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
                self.out.write(b"  sar rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
            }
        }

        // Convert to target width (truncate + sign/zero extend).
        if to_bits < 64 {
            if to_bits <= 32 {
                self.out.write(b"  and eax, ");
                write_u64_hex(self.out, mask_for_bits(to_bits));
                self.out.write(b"\n");
            } else {
                self.out.write(b"  and rax, ");
                write_u64_hex(self.out, mask_for_bits(to_bits));
                self.out.write(b"\n");
            }
            if to_signed {
                let sh = 64u32 - (to_bits as u32);
                self.out.write(b"  shl rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
                self.out.write(b"  sar rax, ");
                write_u32(self.out, sh);
                self.out.write(b"\n");
            }
        } else if to_signed && !from_signed {
            // 64-bit unsigned -> 64-bit signed: keep bits (two's complement).
        }

        // Special-case casts to bool: normalize to 0/1.
        if to_ty == b"bool" && from_ty != b"bool" {
            self.out.write(b"  cmp rax, 0\n");
            self.out.write(b"  setne al\n");
            self.out.write(b"  movzx rax, al\n");
        }

        self.out.write(b"  mov [r15-8], rax\n");
    }

    fn emit_trap_with_loc(&mut self, code: u32, span: Span) {
        if self.debug_trap_loc {
            let (line, _col) = line_col(self.src, span.start);
            self.out.write(b"  mov rdi, ");
            write_u32(self.out, code);
            self.out.write(b"\n");
            self.out.write(b"  mov rsi, 1"); // file_id (hosted single file)
            self.out.write(b"  mov rdx, ");
            write_u32(self.out, line);
            self.out.write(b"\n");
            self.out.write(b"  mov rcx, ");
            write_u32(self.out, self.cur_word_id);
            self.out.write(b"\n");
            self.out.write(b"  jmp __lang_trap_loc\n");
        } else {
            self.out.write(b"  mov rdi, ");
            write_u32(self.out, code);
            self.out.write(b"\n");
            self.out.write(b"  jmp __lang_trap\n");
        }
    }
}

fn fnv1a_u32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn prim_bits_signed(ty: &[u8]) -> Option<(u16, bool)> {
    if ty.starts_with(b"Chan(") {
        return Some((64, false));
    }
    let (bits, signed) = match ty {
        b"u8" => (8, false),
        b"u16" => (16, false),
        b"u32" => (32, false),
        b"u64" => (64, false),
        b"usize" => (64, false),
        b"i8" => (8, true),
        b"i16" => (16, true),
        b"i32" => (32, true),
        b"i64" => (64, true),
        b"isize" => (64, true),
        b"bool" => (8, false),
        b"ptr" | b"ptr_mut" | b"str" | b"mmio" => (64, false),
        _ => return None,
    };
    Some((bits, signed))
}

fn prim_ty_bits_signed(w: &lir::Word, ty: lir::TypeId) -> Option<(u16, bool)> {
    let b = w.types.get(ty.0 as usize).map(|a| a.as_bytes())?;
    prim_bits_signed(b)
}

fn emit_mmio_bounds_check(gen: &mut IrAsmGen<'_>, width: u32, span: Span) {
    const MMIO_SIZE: u32 = 65536;
    let ok = gen.fresh_label();
    let max = MMIO_SIZE.saturating_sub(width);
    gen.out.write(b"  cmp rax, ");
    write_u32(gen.out, max);
    gen.out.write(b"\n");
    gen.out.write(b"  jbe .mmio_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b"\n");
    gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
    gen.out.write(b".mmio_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b":\n");
}

fn emit_mmio_load(gen: &mut IrAsmGen<'_>, width: u32, signed: bool, span: Span) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, width, span);
    match (width, signed) {
        (1, true) => gen.out.write(b"  movsx rax, byte [__mmio_mem + rax]\n"),
        (1, false) => gen.out.write(b"  movzx rax, byte [__mmio_mem + rax]\n"),
        (2, true) => gen.out.write(b"  movsx rax, word [__mmio_mem + rax]\n"),
        (2, false) => gen.out.write(b"  movzx rax, word [__mmio_mem + rax]\n"),
        (4, true) => gen.out.write(b"  movsxd rax, dword [__mmio_mem + rax]\n"),
        (4, false) => gen.out.write(b"  mov eax, dword [__mmio_mem + rax]\n"),
        (8, _) => gen.out.write(b"  mov rax, qword [__mmio_mem + rax]\n"),
        _ => {
            // Shouldn't happen for supported primitive widths; trap if it does.
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return;
        }
    }
    gen.out.write(b"  mov [r15], rax\n");
    gen.out.write(b"  add r15, 8\n");
}

fn emit_mmio_store(gen: &mut IrAsmGen<'_>, width: u32, span: Span) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rcx, [r15]\n");
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, width, span);
    match width {
        1 => gen.out.write(b"  mov byte [__mmio_mem + rax], cl\n"),
        2 => gen.out.write(b"  mov word [__mmio_mem + rax], cx\n"),
        4 => gen.out.write(b"  mov dword [__mmio_mem + rax], ecx\n"),
        8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rcx\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
        }
    }
}

fn emit_mmio_load_field(
    gen: &mut IrAsmGen<'_>,
    reg_width: u32,
    field_bits: u16,
    field_signed: bool,
    mask: u64,
    shift: u8,
    span: Span,
) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, reg_width, span);

    match reg_width {
        1 => gen.out.write(b"  movzx rcx, byte [__mmio_mem + rax]\n"),
        2 => gen.out.write(b"  movzx rcx, word [__mmio_mem + rax]\n"),
        4 => gen.out.write(b"  mov ecx, dword [__mmio_mem + rax]\n"),
        8 => gen.out.write(b"  mov rcx, qword [__mmio_mem + rax]\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return;
        }
    }

    gen.out.write(b"  mov r8, ");
    write_u64_hex(gen.out, mask);
    gen.out.write(b"\n");
    gen.out.write(b"  and rcx, r8\n");
    if shift != 0 {
        gen.out.write(b"  shr rcx, ");
        write_u32(gen.out, shift as u32);
        gen.out.write(b"\n");
    }

    gen.out.write(b"  mov rax, rcx\n");
    if field_bits < 64 {
        if field_bits <= 32 {
            gen.out.write(b"  and eax, ");
            write_u64_hex(gen.out, mask_for_bits(field_bits));
            gen.out.write(b"\n");
        } else {
            gen.out.write(b"  and rax, ");
            write_u64_hex(gen.out, mask_for_bits(field_bits));
            gen.out.write(b"\n");
        }
        if field_signed {
            let sh = 64u32 - (field_bits as u32);
            gen.out.write(b"  shl rax, ");
            write_u32(gen.out, sh);
            gen.out.write(b"\n");
            gen.out.write(b"  sar rax, ");
            write_u32(gen.out, sh);
            gen.out.write(b"\n");
        }
    }

    gen.out.write(b"  mov [r15], rax\n");
    gen.out.write(b"  add r15, 8\n");
}

fn emit_mmio_store_field(gen: &mut IrAsmGen<'_>, reg_width: u32, mask: u64, shift: u8, span: Span) {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rcx, [r15]\n"); // field value
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n"); // addr
    emit_mmio_bounds_check(gen, reg_width, span);

    match reg_width {
        1 => gen.out.write(b"  movzx rdx, byte [__mmio_mem + rax]\n"),
        2 => gen.out.write(b"  movzx rdx, word [__mmio_mem + rax]\n"),
        4 => gen.out.write(b"  mov edx, dword [__mmio_mem + rax]\n"),
        8 => gen.out.write(b"  mov rdx, qword [__mmio_mem + rax]\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return;
        }
    }

    gen.out.write(b"  mov r8, ");
    write_u64_hex(gen.out, mask);
    gen.out.write(b"\n");
    gen.out.write(b"  mov r9, r8\n");
    gen.out.write(b"  not r9\n");
    gen.out.write(b"  and rdx, r9\n");

    gen.out.write(b"  mov r10, rcx\n");
    if shift != 0 {
        gen.out.write(b"  shl r10, ");
        write_u32(gen.out, shift as u32);
        gen.out.write(b"\n");
    }
    gen.out.write(b"  and r10, r8\n");
    gen.out.write(b"  or rdx, r10\n");

    match reg_width {
        1 => gen.out.write(b"  mov byte [__mmio_mem + rax], dl\n"),
        2 => gen.out.write(b"  mov word [__mmio_mem + rax], dx\n"),
        4 => gen.out.write(b"  mov dword [__mmio_mem + rax], edx\n"),
        8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rdx\n"),
        _ => {}
    }
}

fn mask_for_bits(bits: u16) -> u64 {
    if bits >= 64 {
        !0u64
    } else {
        (1u64 << bits) - 1
    }
}

fn max_local_slot_ir(w: &lir::Word) -> Option<u16> {
    let mut max = None;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            let s = match op.kind {
                lir::OpKind::LocalSet { slot, .. } => Some(slot),
                lir::OpKind::LocalGet { slot, .. } => Some(slot),
                _ => None,
            };
            if let Some(s) = s {
                max = Some(match max {
                    Some(m) => core::cmp::max(m, s),
                    None => s,
                });
            }
        }
    }
    max
}

fn locals_bytes_ir(slots: u32) -> u32 {
    if slots == 0 {
        return 0;
    }
    let mut bytes = slots * 8;
    if bytes % 16 != 0 {
        bytes += 8;
    }
    bytes
}

fn write_label(out: &mut dyn Output, name: &[u8]) {
    out.write(b"w_");
    for &b in name {
        let hi = b >> 4;
        let lo = b & 0xf;
        out.write(&[hex_digit(hi), hex_digit(lo)]);
    }
}

fn hex_digit(v: u8) -> u8 {
    match v {
        0..=9 => b'0' + v,
        _ => b'a' + (v - 10),
    }
}

fn emit_push_i64(out: &mut dyn Output, v: i64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov qword [r15], ");
    emit_i64(out, v);
    out.write(b"\n");
    out.write(b"  add r15, 8\n");
}

fn emit_push_u64(out: &mut dyn Output, v: u64) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov qword [r15], ");
    write_u64_hex(out, v);
    out.write(b"\n");
    out.write(b"  add r15, 8\n");
}

fn emit_push_rax(out: &mut dyn Output) {
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_i64(out: &mut dyn Output, mut v: i64) {
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

fn decode_string_bytes(src: &[u8], span: Span) -> Option<FixedVec<u8, 256>> {
    if span.end <= span.start + 1 {
        return None;
    }
    let s = &src[span.start..span.end];
    if s.first().copied()? != b'"' {
        return None;
    }
    if s.last().copied()? != b'"' {
        return None;
    }
    let mut out: FixedVec<u8, 256> = FixedVec::new();
    let mut i = 1usize;
    while i + 1 < s.len() {
        let b = s[i];
        if b == b'\\' {
            i += 1;
            if i + 1 >= s.len() {
                return None;
            }
            let e = s[i];
            let v = match e {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'0' => 0,
                b'\\' => b'\\',
                b'"' => b'"',
                _ => e,
            };
            out.push(v).ok()?;
            i += 1;
            continue;
        }
        out.push(b).ok()?;
        i += 1;
    }
    Some(out)
}

fn emit_dup(out: &mut dyn Output) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  lea rcx, [r15+8]\n");
    out.write(b"  cmp rcx, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_drop(out: &mut dyn Output) {
    out.write(b"  sub r15, 8\n");
}

fn emit_swap(out: &mut dyn Output) {
    out.write(b"  mov rax, [r15-8]\n");
    out.write(b"  mov rcx, [r15-16]\n");
    out.write(b"  mov [r15-8], rcx\n");
    out.write(b"  mov [r15-16], rax\n");
}

fn emit_binop(out: &mut dyn Output, op: &[u8]) {
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

fn emit_cmp(out: &mut dyn Output, setcc: &[u8]) {
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

fn emit_store_local(out: &mut dyn Output, idx: u32) {
    out.write(b"  sub r15, 8\n");
    out.write(b"  mov rax, [r15]\n");
    out.write(b"  mov [rsp+");
    write_u32(out, idx * 8);
    out.write(b"], rax\n");
}

fn emit_load_local(out: &mut dyn Output, idx: u32) {
    out.write(b"  lea rax, [r15+8]\n");
    out.write(b"  cmp rax, r14\n");
    out.write(b"  ja __stack_overflow\n");
    out.write(b"  mov rax, [rsp+");
    write_u32(out, idx * 8);
    out.write(b"]\n");
    out.write(b"  mov [r15], rax\n");
    out.write(b"  add r15, 8\n");
}

fn emit_stack_overflow(out: &mut dyn Output) {
    out.write(b"__stack_overflow:\n");
    out.write(b"  mov rdi, ");
    write_u32(out, lir::trap_code_u32(lir::TrapCode::StackOverflow));
    out.write(b"\n");
    out.write(b"  jmp __lang_trap\n");
}
