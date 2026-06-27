use crate::ophelpers::{fnv1a_u64, slice_span, write_hex, write_sym_label, write_u32};
use crate::RiscVBackend;
use codegen_core::strings::STR_TABLE_CAP;
use codegen_core::{AsmMode, CodegenError};
use frontend::span::Span;
use ir as lir;

fn prim_ty(w: &lir::Word, ty: lir::TypeId) -> Option<lir::Prim> {
    let ty_name = w.types.get(ty.0 as usize).map(|a| a.as_bytes())?;
    lir::Prim::from_type_name(ty_name)
}

fn prim_bits_signed(w: &lir::Word, ty: lir::TypeId) -> Option<(u16, bool)> {
    prim_ty(w, ty).map(|prim| prim.bits_signed(32))
}

fn line_col(src: &[u8], offset: usize) -> (u32, u32) {
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    let end = core::cmp::min(offset, src.len());
    for &b in &src[..end] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

impl<'a> RiscVBackend<'a> {
    pub fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        self.cur_word_id = fnv1a_u64(w.name.as_bytes());
        self.out.write(b"\n");
        if self.mode == AsmMode::Object {
            if is_exported(self.module, self.src, w.name.as_bytes()) {
                let n = lir::Atom::new(w.name.as_bytes()).unwrap_or(lir::AT_EMPTY);
                let idx = self.mi_export_count;
                if idx < self.mi_exports.len() {
                    self.mi_exports[idx] = crate::ModInfoExport {
                        name: n,
                        effects: w.performs.bits(),
                        requires_caps: w.requires.bits(),
                        stack_bound: w.bound.wire_u32(),
                    };
                    self.mi_export_count = idx + 1;
                }
            }
        }
        self.out.write(b"\t.globl ");
        write_sym_label(self.out, w.name.as_bytes());
        self.out.write(b"\n\t.type ");
        write_sym_label(self.out, w.name.as_bytes());
        self.out.write(b", @function\n");
        write_sym_label(self.out, w.name.as_bytes());
        self.out.write(b":\n");

        let slots = max_local_slot(w).map(|m| (m as u32) + 1).unwrap_or(0);
        let locals_bytes = slots * 8;
        let scoped_count = count_scoped_slices(w);
        let scoped_bytes = scoped_count * 8;
        self.scoped_base = locals_bytes;
        self.scoped_slots = scoped_count;
        self.scoped_next = 0;
        // Reserve space for `ra` above the locals.  A word may call another
        // word via `jal`, which clobbers `ra`; without saving it the word's
        // own `ret` returns to the wrong address (self-loop).  `ra` is stored
        // at the top of the frame so local/scoped offsets (measured from `sp`)
        // are unchanged.  Round the frame up to 16 bytes to keep `sp` aligned.
        let local_frame = locals_bytes + scoped_bytes;
        let frame_bytes = (local_frame + 4 + 15) & !15;
        let ra_off = frame_bytes - 4;
        self.out.write(b"\taddi sp, sp, -");
        write_u32(self.out, frame_bytes);
        self.out.write(b"\n\tsw ra, ");
        write_u32(self.out, ra_off);
        self.out.write(b"(sp)\n");
        let base = self.fresh_label();
        // Native-stack-overflow guard: if reserving this frame drove sp below
        // the reserved limit (deep recursion), trap.  __stack_overflow runs in
        // the guard zone below the limit and reports trap_code 10.
        if self.mode == AsmMode::Object {
            self.emit_load_symbol_addr(b"t0", b"__lang_stack_limit");
            self.out.write(b"\tbgeu sp, t0, .Lsk");
            write_u32(self.out, base);
            self.out.write(b"\n\tj __stack_overflow\n.Lsk");
            write_u32(self.out, base);
            self.out.write(b":\n");
        }
        self.out.write(b"\tj .b");
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
        self.out.write(b"\tlw ra, ");
        write_u32(self.out, ra_off);
        self.out.write(b"(sp)\n\taddi sp, sp, ");
        write_u32(self.out, frame_bytes);
        self.out.write(b"\n\tret\n");
        Ok(())
    }

    fn emit_stack_control_ops(
        &mut self,
        _w: &lir::Word,
        op: &lir::Op,
        base: u32,
    ) -> Result<bool, CodegenError> {
        match op.kind {
            lir::OpKind::ConstI64(v) => {
                let low = v as u32;
                let high = (v >> 32) as u32;
                self.emit_const32(low);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_const32(high);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::ConstBool(v) => {
                let val: u32 = if v { 1 } else { 0 };
                self.emit_const32(val);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_const32(0);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::ConstStr(span) => {
                let id = self.intern_str(span)?;
                self.out.write(b"\tla a0, __lang_str_");
                write_u32(self.out, id);
                self.out.write(b"\n\tli a1, 0\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Dup { .. } => {
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Drop { .. } => {
                self.out.write(b"\taddi s2, s2, -8\n");
                Ok(true)
            }
            lir::OpKind::Swap { .. } => {
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out
                    .write(b"\tsw a2, 0(s2)\n\tsw a3, 4(s2)\n\taddi s2, s2, 8\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(true)
            }
            lir::OpKind::AddI64 => {
                self.emit_binop_int(b"add", b"sltu", b"add");
                Ok(true)
            }
            lir::OpKind::SubI64 => {
                self.emit_sub64();
                Ok(true)
            }
            lir::OpKind::MulI64 => {
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tmul a0, a0, a2\n\tli a1, 0\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(true)
            }
            lir::OpKind::Cmp { kind, .. } => {
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                match kind {
                    lir::CmpKind::Eq => self.out.write(b"\txor a0, a0, a2\n\txor a1, a1, a3\n\tor a0, a0, a1\n\tseqz a0, a0\n"),
                    lir::CmpKind::Ne => self.out.write(b"\txor a0, a0, a2\n\txor a1, a1, a3\n\tor a0, a0, a1\n\tsnez a0, a0\n"),
                    lir::CmpKind::Lt => self.out.write(b"\tblt a1, a3, 2f\n\tbgt a1, a3, 3f\n\tbltu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                    lir::CmpKind::Le => self.out.write(b"\tblt a1, a3, 2f\n\tbgt a1, a3, 3f\n\tbleu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                    lir::CmpKind::Gt => self.out.write(b"\tblt a1, a3, 3f\n\tbgt a1, a3, 2f\n\tbgtu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                    lir::CmpKind::Ge => self.out.write(b"\tblt a1, a3, 3f\n\tbgt a1, a3, 2f\n\tbgeu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                }
                self.out
                    .write(b"\tli a1, 0\n\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(true)
            }
            lir::OpKind::AndBool => {
                self.emit_binop_bool(b"and");
                Ok(true)
            }
            lir::OpKind::OrBool => {
                self.emit_binop_bool(b"or");
                Ok(true)
            }
            lir::OpKind::NotBool => {
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tseqz a0, a0\n\tli a1, 0\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(true)
            }
            lir::OpKind::InterruptDisable => {
                self.out.write(b"\tcsrci mstatus, 8\n");
                Ok(true)
            }
            lir::OpKind::InterruptEnable => {
                self.out.write(b"\tcsrsi mstatus, 8\n");
                Ok(true)
            }
            lir::OpKind::LocalSet { slot, .. } => {
                let off = (slot as u32) * 8;
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tsw a0, ");
                write_u32(self.out, off);
                self.out.write(b"(sp)\n\tsw a1, ");
                write_u32(self.out, off + 4);
                self.out.write(b"(sp)\n");
                Ok(true)
            }
            lir::OpKind::LocalGet { slot, .. } => {
                let off = (slot as u32) * 8;
                self.out.write(b"\tlw a0, ");
                write_u32(self.out, off);
                self.out.write(b"(sp)\n\tlw a1, ");
                write_u32(self.out, off + 4);
                self.out.write(b"(sp)\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Call { name, .. } => {
                self.out.write(b"\tjal ");
                write_sym_label(self.out, name.as_bytes());
                self.out.write(b"\n");
                Ok(true)
            }
            lir::OpKind::Br { target } => {
                self.out.write(b"\tj .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, target.0 as u32);
                self.out.write(b"\n");
                Ok(true)
            }
            lir::OpKind::BrIf { then_tgt, else_tgt } => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
                self.out.write(b"\tbnez a0, .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, then_tgt.0 as u32);
                self.out.write(b"\n");
                self.out.write(b"\tj .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, else_tgt.0 as u32);
                self.out.write(b"\n");
                Ok(true)
            }
            lir::OpKind::Ret => {
                self.out.write(b"\tj .endword_");
                write_u32(self.out, base);
                self.out.write(b"\n");
                Ok(true)
            }
            lir::OpKind::TrapIfFalse { code } => {
                let ok = self.fresh_label();
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tbnez a0, .trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b"\n");
                self.emit_trap_with_loc(lir::trap_code_u32(code), op.span);
                self.out.write(b".trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b":\n");
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn emit_memory_ops(
        &mut self,
        _w: &lir::Word,
        op: &lir::Op,
        _base: u32,
    ) -> Result<bool, CodegenError> {
        match op.kind {
            lir::OpKind::Load { ty } => {
                let (bits, signed) = prim_bits_signed(_w, ty)
                    .ok_or(CodegenError::UnsupportedOp { op_name: b"Load" })?;
                // Pop address (low word, discard high)
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
                // Load from address
                match (bits, signed) {
                    (8, true) => self.out.write(b"\tlb a0, 0(a0)\n"),
                    (8, false) => self.out.write(b"\tlbu a0, 0(a0)\n"),
                    (16, true) => self.out.write(b"\tlh a0, 0(a0)\n"),
                    (16, false) => self.out.write(b"\tlhu a0, 0(a0)\n"),
                    (32, _) => self.out.write(b"\tlw a0, 0(a0)\n"),
                    (64, _) => {
                        self.out.write(b"\tlw a0, 0(a0)\n\tlw a1, 4(a0)\n");
                    }
                    _ => return Err(CodegenError::UnsupportedOp { op_name: b"Load" }),
                }
                if bits < 64 {
                    if signed && bits < 32 {
                        // a0 already sign-extended by lb/lh; set a1
                        self.out.write(b"\tsrai a1, a0, 31\n");
                    } else {
                        self.out.write(b"\tli a1, 0\n");
                    }
                }
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Store { ty } => {
                let (bits, _signed) = prim_bits_signed(_w, ty)
                    .ok_or(CodegenError::UnsupportedOp { op_name: b"Store" })?;
                // Pop value (a0:a1), then address
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n");
                match bits {
                    8 => self.out.write(b"\tsb a0, 0(a2)\n"),
                    16 => self.out.write(b"\tsh a0, 0(a2)\n"),
                    32 => self.out.write(b"\tsw a0, 0(a2)\n"),
                    64 => {
                        self.out.write(b"\tsw a0, 0(a2)\n\tsw a1, 4(a2)\n");
                    }
                    _ => return Err(CodegenError::UnsupportedOp { op_name: b"Store" }),
                }
                Ok(true)
            }
            lir::OpKind::AddrOf {
                const_addr: Some(addr),
                ..
            } => {
                let low = addr as u32;
                let high = (addr >> 32) as u32;
                self.emit_const32(low);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_const32(high);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::AddrOf {
                const_addr: None, ..
            } => Err(CodegenError::UnsupportedAddrOf),
            lir::OpKind::PtrAddConst { offset, .. } => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
                if offset <= 2047 {
                    self.out.write(b"\taddi a0, a0, ");
                    write_u32(self.out, offset);
                    self.out.write(b"\n");
                } else {
                    self.out.write(b"\tli a1, ");
                    write_u32(self.out, offset);
                    self.out.write(b"\n\tadd a0, a0, a1\n");
                }
                self.out.write(b"\tli a1, 0\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::PtrAddIndex { scale, .. } => {
                // Pop index (low word), then base (low word)
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n"); // idx
                self.out.write(b"\taddi s2, s2, -8\n\tlw a1, 0(s2)\n"); // base
                if scale == 0 {
                    // base is the result
                } else if scale.is_power_of_two() {
                    let shift = scale.trailing_zeros();
                    self.out.write(b"\tslli a0, a0, ");
                    write_u32(self.out, shift);
                    self.out.write(b"\n");
                    self.out.write(b"\tadd a0, a1, a0\n");
                } else {
                    self.out.write(b"\tli a2, ");
                    write_u32(self.out, scale);
                    self.out.write(b"\n\tmul a0, a0, a2\n\tadd a0, a1, a0\n");
                }
                self.out.write(b"\tli a1, 0\n");
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Cast { from, to } => {
                let from_prim = match prim_ty(_w, from) {
                    Some(prim) => prim,
                    None => return Ok(true),
                };
                let to_prim = match prim_ty(_w, to) {
                    Some(prim) => prim,
                    None => return Ok(true),
                };
                if from_prim == to_prim {
                    return Ok(true);
                }
                let (from_bits, from_signed) = from_prim.bits_signed(32);
                let (to_bits, to_signed) = to_prim.bits_signed(32);
                // Pop value
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                // Mask/sign-extend from source width
                if from_bits < 64 {
                    let mask = ((1u64 << from_bits) - 1) as u32;
                    if mask <= 0xFFFF {
                        self.out.write(b"\tli a2, ");
                        write_hex(self.out, mask);
                        self.out.write(b"\n\tand a0, a0, a2\n");
                    } else {
                        self.out.write(b"\tli a2, ");
                        write_hex(self.out, mask & 0xFFFF);
                        self.out.write(b"\n");
                        self.out.write(b"\tlui a3, ");
                        write_hex(self.out, (mask >> 16) & 0xFFFF);
                        self.out.write(b"\n");
                        self.out.write(b"\tor a2, a2, a3\n\tand a0, a0, a2\n");
                    }
                    if from_signed {
                        let sh = 32 - from_bits;
                        self.out.write(b"\tslli a0, a0, ");
                        write_u32(self.out, sh as u32);
                        self.out.write(b"\n\tsrai a0, a0, ");
                        write_u32(self.out, sh as u32);
                        self.out.write(b"\n");
                        self.out.write(b"\tsrai a1, a0, 31\n");
                    } else {
                        self.out.write(b"\tli a1, 0\n");
                    }
                }
                // Normalize to bool if target is bool
                if to_prim == lir::Prim::Bool && from_prim != lir::Prim::Bool {
                    self.out.write(b"\tsnez a0, a0\n\tli a1, 0\n");
                }
                // Mask/sign-extend to target width
                if to_bits < 64 && to_prim != lir::Prim::Bool {
                    let mask = ((1u64 << to_bits) - 1) as u32;
                    if mask <= 0xFFFF {
                        self.out.write(b"\tli a2, ");
                        write_hex(self.out, mask);
                        self.out.write(b"\n\tand a0, a0, a2\n");
                    } else {
                        self.out.write(b"\tli a2, ");
                        write_hex(self.out, mask & 0xFFFF);
                        self.out.write(b"\n");
                        self.out.write(b"\tlui a3, ");
                        write_hex(self.out, (mask >> 16) & 0xFFFF);
                        self.out.write(b"\n");
                        self.out.write(b"\tor a2, a2, a3\n\tand a0, a0, a2\n");
                    }
                    if to_signed {
                        let sh = 32 - to_bits;
                        self.out.write(b"\tslli a0, a0, ");
                        write_u32(self.out, sh as u32);
                        self.out.write(b"\n\tsrai a0, a0, ");
                        write_u32(self.out, sh as u32);
                        self.out.write(b"\n");
                        self.out.write(b"\tsrai a1, a0, 31\n");
                    } else {
                        self.out.write(b"\tli a1, 0\n");
                    }
                }
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Bitcast { .. } => {
                // No-op: bit pattern unchanged
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn emit_platform_ops(
        &mut self,
        _w: &lir::Word,
        op: &lir::Op,
        _base: u32,
    ) -> Result<(), CodegenError> {
        match op.kind {
            lir::OpKind::TaskSpawn { name, .. } => {
                self.uses_tasks = true;
                self.out.write(b"\tla a0, ");
                write_sym_label(self.out, name.as_bytes());
                self.out.write(b"\n");
                self.out.write(b"\tcall __task_spawn\n");
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.out.write(b"\tsw zero, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::ScopedEnter { ty, len } => {
                let ty_name = _w
                    .types
                    .get(ty.0 as usize)
                    .map(|a| a.as_bytes())
                    .unwrap_or(b"");
                if ty_name.starts_with(b"Slice(") || ty_name.starts_with(b"SliceMut(") {
                    if self.scoped_next >= self.scoped_slots {
                        return Err(CodegenError::ScopedAllocationOverflow);
                    }
                    let slot = self.scoped_next;
                    self.scoped_next = self.scoped_next.wrapping_add(1);
                    let offset = self.scoped_base + (slot * 8);
                    // Pop data pointer from DS (low word), store at [sp+offset]
                    self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
                    self.out.write(b"\tsw a0, ");
                    write_u32(self.out, offset);
                    self.out.write(b"(sp)\n");
                    // Store length at [sp+offset+4]
                    self.out.write(b"\tli a0, ");
                    write_u32(self.out, len);
                    self.out.write(b"\n\tsw a0, ");
                    write_u32(self.out, offset + 4);
                    self.out.write(b"(sp)\n");
                    // Push address of slot as result pointer
                    self.out.write(b"\taddi a0, sp, ");
                    write_u32(self.out, offset);
                    self.out.write(b"\n\tli a1, 0\n");
                    self.out
                        .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                    self.emit_ds_high_update();
                } else if ty_name == b"RegionRef" || ty_name == b"RegionRefMut" {
                    self.out
                        .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                    self.out
                        .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                    self.out
                        .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                    self.emit_ds_high_update();
                }
                Ok(())
            }
            lir::OpKind::MmioPlace { addr, .. } => {
                let low = addr as u32;
                let high = (addr >> 32) as u32;
                self.emit_const32(low);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_const32(high);
                self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::MmioVolLoad { ty, place: _ } => {
                let (bits, _signed) =
                    prim_bits_signed(_w, ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolLoad",
                    })?;
                // Pop address (low word, discard high)
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
                match bits {
                    8 => self.out.write(b"\tlbu a0, 0(a0)\n"),
                    16 => self.out.write(b"\tlhu a0, 0(a0)\n"),
                    32 => self.out.write(b"\tlw a0, 0(a0)\n"),
                    64 => {
                        self.out.write(b"\tlw a0, 0(a0)\n\tlw a1, 4(a0)\n");
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOp {
                            op_name: b"MmioVolLoad",
                        })
                    }
                }
                if bits < 64 {
                    self.out.write(b"\tli a1, 0\n");
                }
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::MmioVolStore {
                ty,
                place: _,
                access: _,
            } => {
                let (bits, _signed) =
                    prim_bits_signed(_w, ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolStore",
                    })?;
                // Pop value (a0:a1), pop address (a2)
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n");
                match bits {
                    8 => self.out.write(b"\tsb a0, 0(a2)\n"),
                    16 => self.out.write(b"\tsh a0, 0(a2)\n"),
                    32 => self.out.write(b"\tsw a0, 0(a2)\n"),
                    64 => {
                        self.out.write(b"\tsw a0, 0(a2)\n\tsw a1, 4(a2)\n");
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOp {
                            op_name: b"MmioVolStore",
                        })
                    }
                }
                Ok(())
            }
            lir::OpKind::MmioVolLoadField {
                reg_ty,
                field_ty,
                place: _,
                mask,
                shift,
            } => {
                let (rbits, _) =
                    prim_bits_signed(_w, reg_ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolLoadField",
                    })?;
                let (fbits, f_signed) =
                    prim_bits_signed(_w, field_ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolLoadField",
                    })?;
                // Pop address
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
                match rbits {
                    32 => self.out.write(b"\tlw a0, 0(a0)\n"),
                    64 => self.out.write(b"\tlw a0, 0(a0)\n\tlw a1, 4(a0)\n"),
                    _ => {
                        return Err(CodegenError::UnsupportedOp {
                            op_name: b"MmioVolLoadField",
                        })
                    }
                }
                // Apply shift (right-shift field to LSB)
                if shift > 0 {
                    self.out.write(b"\tsrli a0, a0, ");
                    write_u32(self.out, shift as u32);
                    self.out.write(b"\n");
                    if rbits == 64 {
                        self.out.write(b"\tslli a1, a1, ");
                        write_u32(self.out, (32 - shift) as u32);
                        self.out.write(b"\n\tslli a1, a1, ");
                        write_u32(self.out, shift as u32);
                        self.out.write(b"\n\tsrli a1, a1, ");
                        write_u32(self.out, shift as u32);
                        self.out.write(b"\n");
                    }
                }
                // Apply mask
                if rbits == 32 {
                    let mask32 = (mask as u32) >> shift;
                    if mask32 != 0xFFFFFFFF {
                        self.out.write(b"\tli a2, ");
                        write_hex(self.out, mask32);
                        self.out.write(b"\n\tand a0, a0, a2\n");
                    }
                }
                // Sign-extend field value if needed
                if fbits < 64 {
                    if f_signed {
                        let sh = 32 - fbits;
                        self.out.write(b"\tslli a0, a0, ");
                        write_u32(self.out, sh as u32);
                        self.out.write(b"\n\tsrai a0, a0, ");
                        write_u32(self.out, sh as u32);
                        self.out.write(b"\n");
                        self.out.write(b"\tsrai a1, a0, 31\n");
                    } else {
                        self.out.write(b"\tli a1, 0\n");
                    }
                }
                self.out
                    .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::MmioVolStoreField {
                reg_ty: _,
                field_ty,
                place: _,
                mask,
                shift,
            } => {
                let (fbits, _) =
                    prim_bits_signed(_w, field_ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolStoreField",
                    })?;
                // Pop value (a0 = low word), pop address (a2)
                self.out
                    .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                let _ = fbits; // suppress warning; value in a0 (a1 is the string literal, not a rust variable)
                self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n");
                // Load current register value
                self.out.write(b"\tlw a3, 0(a2)\n");
                // Clear field bits
                let shifted_mask = mask.wrapping_shl(shift as u32) & 0xFFFFFFFF;
                let clear = (!shifted_mask) & 0xFFFFFFFF;
                self.out.write(b"\tli a1, ");
                write_hex(self.out, clear as u32);
                self.out.write(b"\n\tand a3, a3, a1\n");
                // Shift value to field position and OR
                if shift > 0 {
                    self.out.write(b"\tslli a0, a0, ");
                    write_u32(self.out, shift as u32);
                    self.out.write(b"\n");
                }
                self.out.write(b"\tor a3, a3, a0\n\tsw a3, 0(a2)\n");
                Ok(())
            }
            lir::OpKind::CheckSubtype { .. } => Err(CodegenError::UnsupportedCheckSubtype),
            _ => Err(CodegenError::UnsupportedOp {
                op_name: b"emit_op",
            }),
        }
    }

    fn emit_op(&mut self, _w: &lir::Word, op: &lir::Op, base: u32) -> Result<(), CodegenError> {
        if self.emit_stack_control_ops(_w, op, base)? {
            return Ok(());
        }
        if self.emit_memory_ops(_w, op, base)? {
            return Ok(());
        }
        self.emit_platform_ops(_w, op, base)
    }

    fn emit_const32(&mut self, val: u32) {
        if val == 0 {
            self.out.write(b"\tli a0, 0\n");
        } else {
            self.out.write(b"\tli a0, ");
            write_hex(self.out, val);
            self.out.write(b"\n");
        }
    }

    /// Update __lang_ds_high if s2 exceeds the stored value.
    /// Preserves all registers (uses t0/t1 which are caller-save).
    fn emit_ds_high_update(&mut self) {
        let id = self.fresh_label();
        self.emit_load_symbol_addr(b"t0", b"__lang_ds_high");
        self.out.write(b"\tlw t1, 0(t0)\n");
        self.out.write(b"\tbltu s2, t1, .ds_high_");
        write_u32(self.out, id);
        self.out.write(b"\n\tsw s2, 0(t0)\n");
        self.out.write(b".ds_high_");
        write_u32(self.out, id);
        self.out.write(b":\n");
    }

    fn emit_load_symbol_addr(&mut self, reg: &[u8], sym: &[u8]) {
        let id = self.fresh_label();
        self.out.write(b".Laddr_load_");
        write_u32(self.out, id);
        self.out.write(b":\n\tauipc ");
        self.out.write(reg);
        self.out.write(b", %pcrel_hi(.Laddr_word_");
        write_u32(self.out, id);
        self.out.write(b")\n\tlw ");
        self.out.write(reg);
        self.out.write(b", %pcrel_lo(.Laddr_load_");
        write_u32(self.out, id);
        self.out.write(b")(");
        self.out.write(reg);
        self.out.write(b")\n\tj .Laddr_after_");
        write_u32(self.out, id);
        self.out.write(b"\n\t.balign 4\n.Laddr_word_");
        write_u32(self.out, id);
        self.out.write(b":\n\t.word ");
        self.out.write(sym);
        self.out.write(b"\n.Laddr_after_");
        write_u32(self.out, id);
        self.out.write(b":\n");
    }

    /// 64-bit subtraction with correct borrow detection.
    /// Must detect borrow (a0_lo < a2_lo) BEFORE modifying a0.
    fn emit_sub64(&mut self) {
        self.out
            .write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
        self.out
            .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
        // borrow = (a0 < a2) unsigned — detect BEFORE sub
        self.out.write(b"\tsltu a4, a0, a2\n");
        self.out.write(b"\tsub a0, a0, a2\n");
        self.out.write(b"\tsub a1, a1, a3\n");
        self.out.write(b"\tsub a1, a1, a4\n");
        self.out
            .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
    }

    fn emit_binop_int(&mut self, lo: &[u8], _cmp: &[u8], hi: &[u8]) {
        self.out
            .write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
        self.out
            .write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
        // 64-bit: lo + carry detection + hi adjustment
        self.out.write(b"\t");
        self.out.write(lo);
        self.out.write(b" a0, a0, a2\n");
        // Carry = (unsigned a0_before < unsigned a2) for add, or (a0_before < a2) for sub
        self.out.write(b"\tsltu a4, a0, a2\n");
        self.out.write(b"\t");
        self.out.write(hi);
        self.out.write(b" a1, a1, a3\n");
        self.out.write(b"\tadd a1, a1, a4\n");
        self.out
            .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
    }

    fn emit_binop_bool(&mut self, insn: &[u8]) {
        self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n");
        self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
        self.out.write(b"\t");
        self.out.write(insn);
        self.out.write(b" a0, a0, a2\n");
        self.out.write(b"\tsnez a0, a0\n\tli a1, 0\n");
        self.out
            .write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
    }

    // ---- Trap ----

    fn emit_trap_with_loc(&mut self, code: u32, span: Span) {
        if self.debug_trap_loc {
            // Register contract for __lang_trap_loc (RISC-V):
            //   a0 = trap_code
            //   a1 = valid (1)
            //   a2 = source_line
            //   a3 = word_hash low 32 bits
            //   a4 = word_hash high 32 bits
            let wh = self.cur_word_id;
            let (line, _col) = line_col(self.src, span.start);

            self.out.write(b"\tli a0, ");
            write_hex(self.out, code);
            self.out.write(b"\n");
            self.out.write(b"\tli a1, 1\n");
            self.out.write(b"\tli a2, ");
            write_u32(self.out, line);
            self.out.write(b"\n");
            let lo = wh as u32;
            let hi = (wh >> 32) as u32;
            self.out.write(b"\tli a3, ");
            write_hex(self.out, lo);
            self.out.write(b"\n");
            self.out.write(b"\tli a4, ");
            write_hex(self.out, hi);
            self.out.write(b"\n");
            self.out.write(b"\tj __lang_trap_loc\n");
        } else {
            self.out.write(b"\tli a0, ");
            write_hex(self.out, code);
            self.out.write(b"\n\tj __lang_trap\n");
        }
    }

    // ---- String interning ----

    fn intern_str(&mut self, span: Span) -> Result<u32, CodegenError> {
        for i in 0..self.str_len {
            if slice_span(self.src, self.str_spans[i]) == slice_span(self.src, span) {
                return Ok(self.str_ids[i]);
            }
        }
        if self.str_len >= STR_TABLE_CAP {
            return Err(CodegenError::StringLiteralCapacityExceeded);
        }
        let id = self.fresh_label();
        self.str_spans[self.str_len] = span;
        self.str_ids[self.str_len] = id;
        self.str_len += 1;
        Ok(id)
    }
}

fn is_exported(module: &frontend::parse::ModuleAst, src: &[u8], name: &[u8]) -> bool {
    for export in module.exports.iter() {
        if slice_span(src, *export) == name {
            return true;
        }
    }
    false
}

fn max_local_slot(w: &lir::Word) -> Option<u16> {
    let mut max: Option<u16> = None;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            if let lir::OpKind::LocalSet { slot, .. } = op.kind {
                if max.map_or(true, |m| slot > m) {
                    max = Some(slot);
                }
            }
        }
    }
    max
}

fn count_scoped_slices(w: &lir::Word) -> u32 {
    let mut count = 0u32;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            if let lir::OpKind::ScopedEnter { ty, .. } = op.kind {
                let name = w
                    .types
                    .get(ty.0 as usize)
                    .map(|a| a.as_bytes())
                    .unwrap_or(b"");
                if name.starts_with(b"Slice(") || name.starts_with(b"SliceMut(") {
                    count = count.wrapping_add(1);
                }
            }
        }
    }
    count
}
