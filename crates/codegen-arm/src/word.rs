use codegen_core::{AsmMode, CodegenError};
use frontend::span::Span;
use ir as lir;

use crate::ophelpers::{fnv1a_u64, slice_span, write_hex, write_sym_label, write_u32};
use crate::ArmThumbBackend;

impl<'a> ArmThumbBackend<'a> {
    pub fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        self.cur_word_id = fnv1a_u64(w.name.as_bytes()) as u32;
        self.out.write(b"\n");

        if self.mode == AsmMode::Object {
            let exported = is_exported(self.module, self.src, w.name.as_bytes());
            if exported {
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

        self.out.write(b"\t.thumb_func\n");
        self.out.write(b"\t.global ");
        write_sym_label(self.out, w.name.as_bytes());
        self.out.write(b"\n\t.type ");
        write_sym_label(self.out, w.name.as_bytes());
        self.out.write(b", %function\n");
        write_sym_label(self.out, w.name.as_bytes());
        self.out.write(b":\n");

        let slots = max_local_slot(w).map(|m| (m as u32) + 1).unwrap_or(0);
        let frame_bytes = slots * 8;
        if frame_bytes > 0 {
            self.out.write(b"\tsub sp, sp, #");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }

        let base = self.fresh_label();
        self.out.write(b"\tb .b");
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
            self.out.write(b"\tadd sp, sp, #");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }
        self.out.write(b"\tbx lr\n");
        Ok(())
    }

    fn emit_op(&mut self, _w: &lir::Word, op: &lir::Op, base: u32) -> Result<(), CodegenError> {
        match op.kind {
            lir::OpKind::ConstI64(v) => {
                let low = v as u32;
                let high = (v >> 32) as u32;
                self.emit_const32(low);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_const32(high);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::ConstBool(v) => {
                let val: u32 = if v { 1 } else { 0 };
                self.emit_const32(val);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_const32(0);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                Ok(())
            }
            lir::OpKind::ConstStr(_) => {
                return Err(CodegenError::UnsupportedOp { op_name: b"ConstStr" });
            }
            lir::OpKind::Dup { .. } => {
                self.out.write(b"\tsubs r4, r4, #8\n");
                self.out.write(b"\tldrd r0, r1, [r4]\n");
                self.out.write(b"\tstrd r0, r1, [r4]\n");
                self.out.write(b"\tadds r4, r4, #8\n");
                self.out.write(b"\tstrd r0, r1, [r4]\n");
                self.out.write(b"\tadds r4, r4, #8\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::Drop { .. } => {
                self.out.write(b"\tsubs r4, r4, #8\n");
                Ok(())
            }
            lir::OpKind::Swap { .. } => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.emit_push_r2r3();
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::AddI64 => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.out.write(b"\tadds r0, r0, r2\n\tadc r1, r1, r3\n");
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::SubI64 => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.out.write(b"\tsubs r0, r0, r2\n\tsbc r1, r1, r3\n");
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::MulI64 => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.out.write(b"\tmuls r0, r2, r0\n\teors r1, r1\n");
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::Cmp { kind, .. } => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                match kind {
                    lir::CmpKind::Eq => {
                        self.out.write(b"\tcmp r0, r2\n");
                        self.out.write(b"\titt eq\n\tcmpeq r1, r3\n");
                        self.out.write(b"\tite eq\n\tmoveq r0, #1\n\tmovne r0, #0\n");
                    }
                    lir::CmpKind::Ne => {
                        self.out.write(b"\tcmp r0, r2\n");
                        self.out.write(b"\titt ne\n\tcmpne r1, r3\n");
                        self.out.write(b"\tite ne\n\tmovne r0, #1\n\tmoveq r0, #0\n");
                    }
                    lir::CmpKind::Lt => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out.write(b"\tite lt\n\tmovlt r0, #1\n\tmovge r0, #0\n");
                    }
                    lir::CmpKind::Le => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out.write(b"\tite le\n\tmovle r0, #1\n\tmovgt r0, #0\n");
                    }
                    lir::CmpKind::Gt => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out.write(b"\tite gt\n\tmovgt r0, #1\n\tmovle r0, #0\n");
                    }
                    lir::CmpKind::Ge => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out.write(b"\tite ge\n\tmovge r0, #1\n\tmovlt r0, #0\n");
                    }
                }
                self.out.write(b"\teors r1, r1\n");
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::AndBool => {
                self.emit_pop_one_r2();
                self.emit_pop_one_r0();
                self.out.write(b"\tands r0, r0, r2\n");
                self.emit_bool_normalize();
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::OrBool => {
                self.emit_pop_one_r2();
                self.emit_pop_one_r0();
                self.out.write(b"\torrs r0, r0, r2\n");
                self.emit_bool_normalize();
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::NotBool => {
                self.emit_pop_two_r0r1();
                self.out.write(b"\tcmp r0, #0\n");
                self.out.write(b"\titte ne\n\tmovne r0, #0\n\tmoveq r0, #1\n");
                self.out.write(b"\teors r1, r1\n");
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::LocalSet { slot, .. } => {
                let offset = (slot as u32) * 8;
                self.out.write(b"\tsubs r4, r4, #8\n");
                self.out.write(b"\tldrd r0, r1, [r4]\n");
                self.out.write(b"\tstrd r0, r1, [sp, #");
                write_u32(self.out, offset);
                self.out.write(b"]\n");
                Ok(())
            }
            lir::OpKind::LocalGet { slot, .. } => {
                let offset = (slot as u32) * 8;
                self.out.write(b"\tldrd r0, r1, [sp, #");
                write_u32(self.out, offset);
                self.out.write(b"]\n");
                self.emit_push_r0r1();
                Ok(())
            }
            lir::OpKind::Call { name, .. } => {
                self.out.write(b"\tbl ");
                write_sym_label(self.out, name.as_bytes());
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::Br { target } => {
                self.out.write(b"\tb .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, target.0 as u32);
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::BrIf { then_tgt, else_tgt } => {
                self.out.write(b"\tsubs r4, r4, #8\n");
                self.out.write(b"\tldr r0, [r4]\n");
                self.out.write(b"\tcmp r0, #0\n");
                self.out.write(b"\tbne .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, then_tgt.0 as u32);
                self.out.write(b"\n\tb .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, else_tgt.0 as u32);
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::Ret => {
                self.out.write(b"\tb .endword_");
                write_u32(self.out, base);
                self.out.write(b"\n");
                Ok(())
            }
            lir::OpKind::TrapIfFalse { code } => {
                let ok = self.fresh_label();
                self.out.write(b"\tsubs r4, r4, #8\n");
                self.out.write(b"\tldr r0, [r4]\n");
                self.out.write(b"\tcmp r0, #0\n");
                self.out.write(b"\tbne .trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b"\n");
                self.emit_trap_with_loc(lir::trap_code_u32(code), op.span);
                self.out.write(b".trap_ok_");
                write_u32(self.out, ok);
                self.out.write(b":\n");
                Ok(())
            }
            _ => Err(CodegenError::UnsupportedOp {
                op_name: b"ARM unsupported op",
            }),
        }
    }

    // ---- 32-bit constant loading ----

    /// Load a 32-bit unsigned value into r0.
    fn emit_const32(&mut self, val: u32) {
        if val == 0 {
            self.out.write(b"\teors r0, r0\n");
        } else if val <= 255 {
            self.out.write(b"\tmovs r0, #");
            write_u32(self.out, val);
            self.out.write(b"\n");
        } else {
            self.out.write(b"\tldr r0, =");
            write_hex(self.out, val as u64);
            self.out.write(b"\n");
        }
    }

    // ---- DS high-water update ----

    /// Update __lang_ds_high if r4 exceeds the stored value.
    /// Preserves r0-r3 by push/pop.
    fn emit_ds_high_update(&mut self) {
        let id = self.fresh_label();
        self.out.write(b"\tpush {r0, r1}\n");
        self.out.write(b"\tldr r1, =__lang_ds_high\n");
        self.out.write(b"\tldr r0, [r1]\n");
        self.out.write(b"\tcmp r4, r0\n");
        self.out.write(b"\tbls .ds_high_");
        write_u32(self.out, id);
        self.out.write(b"\n\tstr r4, [r1]\n");
        self.out.write(b".ds_high_");
        write_u32(self.out, id);
        self.out.write(b":\n\tpop {r0, r1}\n");
    }

    // ---- DS push/pop helpers ----

    /// Pop one i32 from DS into r0 (low word only, discards high word).
    fn emit_pop_one_r0(&mut self) {
        self.out.write(b"\tsubs r4, r4, #8\n\tldr r0, [r4]\n");
    }

    fn emit_pop_one_r2(&mut self) {
        self.out.write(b"\tsubs r4, r4, #8\n\tldr r2, [r4]\n");
    }

    /// Pop i64 from DS into r0:r1 (low, high).
    fn emit_pop_two_r0r1(&mut self) {
        self.out.write(b"\tsubs r4, r4, #8\n\tldrd r0, r1, [r4]\n");
    }

    /// Pop i64 from DS into r2:r3 (low, high).
    fn emit_pop_two_r2r3(&mut self) {
        self.out.write(b"\tsubs r4, r4, #8\n\tldrd r2, r3, [r4]\n");
    }

    /// Push i64 from r0:r1 onto DS.
    fn emit_push_r0r1(&mut self) {
        self.out.write(b"\tstrd r0, r1, [r4]\n\tadds r4, r4, #8\n");
    }

    /// Push i64 from r2:r3 onto DS.
    fn emit_push_r2r3(&mut self) {
        self.out.write(b"\tstrd r2, r3, [r4]\n\tadds r4, r4, #8\n");
    }

    // ---- Bool helpers ----

    /// Normalize r0 to 0/1 after a boolean ALU op.
    fn emit_bool_normalize(&mut self) {
        self.out.write(b"\tcmp r0, #0\n");
        self.out.write(b"\titte ne\n\tmovne r0, #1\n\tmoveq r0, #0\n");
    }

    // ---- Trap ----

    fn emit_trap_with_loc(&mut self, code: u32, _span: Span) {
        self.out.write(b"\tmovs r0, #");
        write_u32(self.out, code);
        self.out.write(b"\n\tb __lang_trap\n");
    }
}

fn is_exported(module: &frontend::parse::ModuleAst, src: &[u8], name: &[u8]) -> bool {
    for export in module.exports.iter() {
        let ename = slice_span(src, *export);
        if ename == name {
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
