use codegen_core::{AsmMode, CodegenError};
use ir as lir;
use crate::ophelpers::{fnv1a_u64, slice_span, write_hex, write_sym_label, write_u32};
use crate::RiscVBackend;

impl<'a> RiscVBackend<'a> {
    pub fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        self.cur_word_id = fnv1a_u64(w.name.as_bytes()) as u32;
        self.out.write(b"\n");
        if self.mode == AsmMode::Object {
            if is_exported(self.module, self.src, w.name.as_bytes()) {
                let n = lir::Atom::new(w.name.as_bytes()).unwrap_or(lir::AT_EMPTY);
                let idx = self.mi_export_count;
                if idx < self.mi_exports.len() {
                    self.mi_exports[idx] = crate::ModInfoExport { name: n, effects: w.performs.bits(), requires_caps: w.requires.bits(), stack_bound: w.bound.wire_u32() };
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
        let frame_bytes = slots * 8;
        if frame_bytes > 0 {
            self.out.write(b"\taddi sp, sp, -");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }
        let base = self.fresh_label();
        self.out.write(b"\tj .b"); write_u32(self.out, base); self.out.write(b"_"); write_u32(self.out, w.entry.0 as u32); self.out.write(b"\n");

        for b in w.blocks.iter() {
            self.out.write(b".b"); write_u32(self.out, base); self.out.write(b"_"); write_u32(self.out, b.id.0 as u32); self.out.write(b":\n");
            for op in b.ops.iter() { self.emit_op(w, op, base)?; }
        }

        self.out.write(b".endword_"); write_u32(self.out, base); self.out.write(b":\n");
        if frame_bytes > 0 { self.out.write(b"\taddi sp, sp, "); write_u32(self.out, frame_bytes); self.out.write(b"\n"); }
        self.out.write(b"\tret\n");
        Ok(())
    }

    fn emit_op(&mut self, _w: &lir::Word, op: &lir::Op, base: u32) -> Result<(), CodegenError> {
        match op.kind {
            lir::OpKind::ConstI64(v) => {
                let low = v as u32; let high = (v >> 32) as u32;
                self.emit_const32(low); self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_const32(high); self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::ConstBool(v) => {
                let val: u32 = if v { 1 } else { 0 };
                self.emit_const32(val); self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_const32(0); self.out.write(b"\tsw a0, 0(s2)\n\taddi s2, s2, 4\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::ConstStr(_) => Err(CodegenError::UnsupportedOp { op_name: b"ConstStr" }),
            lir::OpKind::Dup { .. } => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::Drop { .. } => { self.out.write(b"\taddi s2, s2, -8\n"); Ok(()) }
            lir::OpKind::Swap { .. } => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tsw a2, 0(s2)\n\tsw a3, 4(s2)\n\taddi s2, s2, 8\n");
                self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(())
            }
            lir::OpKind::AddI64 => { self.emit_binop_int(b"add", b"sltu", b"add"); Ok(()) }
            lir::OpKind::SubI64 => { self.emit_sub64(); Ok(()) }
            lir::OpKind::MulI64 => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tmul a0, a0, a2\n\tli a1, 0\n");
                self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(())
            }
            lir::OpKind::Cmp { kind, .. } => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                match kind {
                    lir::CmpKind::Eq => self.out.write(b"\txor a0, a0, a2\n\txor a1, a1, a3\n\tor a0, a0, a1\n\tseqz a0, a0\n"),
                    lir::CmpKind::Ne => self.out.write(b"\txor a0, a0, a2\n\txor a1, a1, a3\n\tor a0, a0, a1\n\tsnez a0, a0\n"),
                    lir::CmpKind::Lt => self.out.write(b"\tblt a1, a3, 2f\n\tbgt a1, a3, 3f\n\tbltu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                    lir::CmpKind::Le => self.out.write(b"\tblt a1, a3, 2f\n\tbgt a1, a3, 3f\n\tbleu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                    lir::CmpKind::Gt => self.out.write(b"\tblt a1, a3, 3f\n\tbgt a1, a3, 2f\n\tbgtu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                    lir::CmpKind::Ge => self.out.write(b"\tblt a1, a3, 3f\n\tbgt a1, a3, 2f\n\tbgeu a0, a2, 2f\n3:\tli a0, 0\n\tj 4f\n2:\tli a0, 1\n4:\n"),
                }
                self.out.write(b"\tli a1, 0\n\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(())
            }
            lir::OpKind::AndBool => { self.emit_binop_bool(b"and"); Ok(()) }
            lir::OpKind::OrBool => { self.emit_binop_bool(b"or"); Ok(()) }
            lir::OpKind::NotBool => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tseqz a0, a0\n\tli a1, 0\n");
                self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                Ok(())
            }
            lir::OpKind::LocalSet { slot, .. } => {
                let off = (slot as u32) * 8;
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
                self.out.write(b"\tsw a0, "); write_u32(self.out, off); self.out.write(b"(sp)\n\tsw a1, "); write_u32(self.out, off + 4); self.out.write(b"(sp)\n");
                Ok(())
            }
            lir::OpKind::LocalGet { slot, .. } => {
                let off = (slot as u32) * 8;
                self.out.write(b"\tlw a0, "); write_u32(self.out, off); self.out.write(b"(sp)\n\tlw a1, "); write_u32(self.out, off + 4); self.out.write(b"(sp)\n");
                self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::Call { name, .. } => {
                self.out.write(b"\tjal "); write_sym_label(self.out, name.as_bytes()); self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::Br { target } => {
                self.out.write(b"\tj .b"); write_u32(self.out, base); self.out.write(b"_"); write_u32(self.out, target.0 as u32); self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::BrIf { then_tgt, else_tgt } => {
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
                self.out.write(b"\tbnez a0, .b"); write_u32(self.out, base); self.out.write(b"_"); write_u32(self.out, then_tgt.0 as u32); self.out.write(b"\n");
                self.out.write(b"\tj .b"); write_u32(self.out, base); self.out.write(b"_"); write_u32(self.out, else_tgt.0 as u32); self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::Ret => { self.out.write(b"\tj .endword_"); write_u32(self.out, base); self.out.write(b"\n"); Ok(()) }
            lir::OpKind::TrapIfFalse { code } => {
                let ok = self.fresh_label();
                self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tbnez a0, .trap_ok_"); write_u32(self.out, ok); self.out.write(b"\n");
                self.out.write(b"\tli a0, "); write_u32(self.out, lir::trap_code_u32(code)); self.out.write(b"\n\tj __lang_trap\n");
                self.out.write(b".trap_ok_"); write_u32(self.out, ok); self.out.write(b":\n");
                Ok(())
            }
            _ => Err(CodegenError::UnsupportedOp { op_name: b"RISC-V unsupported op" }),
        }
    }

    fn emit_const32(&mut self, val: u32) {
        if val == 0 { self.out.write(b"\tli a0, 0\n"); }
        else { self.out.write(b"\tli a0, "); write_hex(self.out, val); self.out.write(b"\n"); }
    }

    /// Update __lang_ds_high if s2 exceeds the stored value.
    /// Preserves all registers (uses t0/t1 which are caller-save).
    fn emit_ds_high_update(&mut self) {
        let id = self.fresh_label();
        self.out.write(b"\tla t0, __lang_ds_high\n");
        self.out.write(b"\tlw t1, 0(t0)\n");
        self.out.write(b"\tbltu s2, t1, .ds_high_");
        write_u32(self.out, id);
        self.out.write(b"\n\tsw s2, 0(t0)\n");
        self.out.write(b".ds_high_");
        write_u32(self.out, id);
        self.out.write(b":\n");
    }

    /// 64-bit subtraction with correct borrow detection.
    /// Must detect borrow (a0_lo < a2_lo) BEFORE modifying a0.
    fn emit_sub64(&mut self) {
        self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
        self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
        // borrow = (a0 < a2) unsigned — detect BEFORE sub
        self.out.write(b"\tsltu a4, a0, a2\n");
        self.out.write(b"\tsub a0, a0, a2\n");
        self.out.write(b"\tsub a1, a1, a3\n");
        self.out.write(b"\tsub a1, a1, a4\n");
        self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
    }

    fn emit_binop_int(&mut self, lo: &[u8], _cmp: &[u8], hi: &[u8]) {
        self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n\tlw a3, 4(s2)\n");
        self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n\tlw a1, 4(s2)\n");
        // 64-bit: lo + carry detection + hi adjustment
        self.out.write(b"\t"); self.out.write(lo); self.out.write(b" a0, a0, a2\n");
        // Carry = (unsigned a0_before < unsigned a2) for add, or (a0_before < a2) for sub
        self.out.write(b"\tsltu a4, a0, a2\n");
        self.out.write(b"\t"); self.out.write(hi); self.out.write(b" a1, a1, a3\n");
        self.out.write(b"\tadd a1, a1, a4\n");
        self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
    }

    fn emit_binop_bool(&mut self, insn: &[u8]) {
        self.out.write(b"\taddi s2, s2, -8\n\tlw a2, 0(s2)\n");
        self.out.write(b"\taddi s2, s2, -8\n\tlw a0, 0(s2)\n");
        self.out.write(b"\t"); self.out.write(insn); self.out.write(b" a0, a0, a2\n");
        self.out.write(b"\tsnez a0, a0\n\tli a1, 0\n");
        self.out.write(b"\tsw a0, 0(s2)\n\tsw a1, 4(s2)\n\taddi s2, s2, 8\n");
    }
}

fn is_exported(module: &frontend::parse::ModuleAst, src: &[u8], name: &[u8]) -> bool {
    for export in module.exports.iter() { if slice_span(src, *export) == name { return true; } }
    false
}

fn max_local_slot(w: &lir::Word) -> Option<u16> {
    let mut max: Option<u16> = None;
    for b in w.blocks.iter() { for op in b.ops.iter() { if let lir::OpKind::LocalSet { slot, .. } = op.kind { if max.map_or(true, |m| slot > m) { max = Some(slot); } } } }
    max
}
