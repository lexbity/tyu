use codegen_core::strings::STR_TABLE_CAP;
use codegen_core::{AsmMode, CodegenError};
use frontend::{parse::DeclKind, span::Span};
use ir as lir;

use crate::ophelpers::{
    fnv1a_u64, slice_span, write_hex, write_res_label, write_sym_label, write_u32,
};
use crate::ArmThumbBackend;

fn prim_ty(w: &lir::Word, ty: lir::TypeId) -> Option<lir::Prim> {
    let ty_name = w.types.get(ty.0 as usize).map(|a| a.as_bytes())?;
    lir::Prim::from_type_name(ty_name)
}

fn prim_bits_signed(w: &lir::Word, ty: lir::TypeId) -> Option<(u16, bool)> {
    prim_ty(w, ty).map(|prim| prim.bits_signed(32))
}

fn find_word_decl<'a>(
    module: &'a frontend::parse::ModuleAst,
    src: &'a [u8],
    name: &[u8],
) -> Option<&'a frontend::parse::DeclAst> {
    for d in module.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        let dname = slice_span(src, d.name);
        if dname == name {
            return Some(d);
        }
    }
    None
}

fn find_resource_decl<'a>(
    module: &'a frontend::parse::ModuleAst,
    src: &'a [u8],
    name: &[u8],
) -> Option<&'a frontend::parse::DeclAst> {
    for d in module.decls.iter() {
        if d.kind != DeclKind::Resource {
            continue;
        }
        let dname = slice_span(src, d.name);
        if dname == name {
            return Some(d);
        }
    }
    None
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

/// Write an ARM register name (r0-r12) to the output stream.
fn write_reg(out: &mut dyn frontend::parse::Output, reg: u8) {
    if reg >= 10 {
        out.write(&[b'1', b'0' + reg - 10]);
    } else {
        out.write(&[b'0' + reg]);
    }
}

/// Emit a 32-bit immediate load into the given ARM register.
/// Handles 0, small immediates (movs), and large values (literal pool).
fn emit_thumb_mov32(out: &mut dyn frontend::parse::Output, reg: u8, val: u32) {
    match val {
        0 => {
            out.write(b"\teors r");
            write_reg(out, reg);
            out.write(b", r");
            write_reg(out, reg);
            out.write(b"\n");
        }
        1..=255 => {
            out.write(b"\tmovs r");
            write_reg(out, reg);
            out.write(b", #");
            write_u32(out, val);
            out.write(b"\n");
        }
        _ => {
            out.write(b"\tldr r");
            write_reg(out, reg);
            out.write(b", =");
            write_hex(out, val as u64);
            out.write(b"\n");
        }
    }
}

impl<'a> ArmThumbBackend<'a> {
    pub fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        self.cur_word_id = fnv1a_u64(w.name.as_bytes());
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

        if self.mode == AsmMode::Object {
            if let Some(decl) = find_word_decl(self.module, self.src, w.name.as_bytes()) {
                for attr in decl.attrs.iter() {
                    if let frontend::parse::AttrAst::Interrupt { vector } = attr {
                        let vec_name = slice_span(self.src, *vector);
                        if vec_name != b"SysTick" {
                            return Err(CodegenError::UnsupportedOp {
                                op_name: b"@interrupt",
                            });
                        }
                        self.out.write(b"\t.global __lang_systick_handler\n");
                        self.out.write(b"\t.thumb_set __lang_systick_handler, ");
                        write_sym_label(self.out, w.name.as_bytes());
                        self.out.write(b"\n");
                    }
                }
            }
        }

        let slots = max_local_slot(w).map(|m| (m as u32) + 1).unwrap_or(0);
        let locals_bytes = slots * 8;
        let scoped_count = count_scoped_slices(w);
        let scoped_bytes = scoped_count * 8; // each slot: ptr(4) + len(4)
        self.scoped_base = locals_bytes;
        self.scoped_slots = scoped_count;
        self.scoped_next = 0;
        // Reserve space for `lr` above the locals.  A word may call another
        // word via `bl`, which clobbers `lr`; without saving it the word's own
        // `bx lr` returns to the wrong address (self-loop).  `lr` is stored at
        // the top of the frame so local/scoped offsets (measured from `sp`) are
        // unchanged.  `locals_bytes`/`scoped_bytes` are multiples of 8, so
        // `local_frame + 8` keeps `sp` 8-byte aligned (AAPCS).
        let local_frame = locals_bytes + scoped_bytes;
        let frame_bytes = local_frame + 8;
        let lr_off = local_frame + 4;
        self.out.write(b"\tsub sp, sp, #");
        write_u32(self.out, frame_bytes);
        self.out.write(b"\n\tstr lr, [sp, #");
        write_u32(self.out, lr_off);
        self.out.write(b"]\n");

        let base = self.fresh_label();
        // Native-stack-overflow guard: if reserving this frame drove sp below
        // the reserved limit (deep recursion), trap.  __stack_overflow runs in
        // the guard zone below the limit and reports trap_code 10.
        if self.mode == AsmMode::Object {
            self.out.write(b"\tldr ip, =__lang_stack_limit\n");
            self.out.write(b"\tcmp sp, ip\n");
            self.out.write(b"\tbhs .Lsk");
            write_u32(self.out, base);
            self.out.write(b"\n\tb __stack_overflow\n.Lsk");
            write_u32(self.out, base);
            self.out.write(b":\n");
        }
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
        self.out.write(b"\tldr lr, [sp, #");
        write_u32(self.out, lr_off);
        self.out.write(b"]\n\tadd sp, sp, #");
        write_u32(self.out, frame_bytes);
        self.out.write(b"\n\tbx lr\n");
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
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_const32(high);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::ConstBool(v) => {
                let val: u32 = if v { 1 } else { 0 };
                self.emit_const32(val);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_const32(0);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::ConstStr(span) => {
                let id = self.intern_str(span)?;
                self.out.write(b"\tldr r0, =__lang_str_");
                write_u32(self.out, id);
                self.out.write(b"\n");
                self.out.write(b"\teors r1, r1\n");
                self.emit_push_r0r1();
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Dup { .. } => {
                self.out.write(b"\tsubs r4, r4, #8\n");
                self.out.write(b"\tldrd r0, r1, [r4]\n");
                self.out.write(b"\tstrd r0, r1, [r4]\n");
                self.out.write(b"\tadds r4, r4, #8\n");
                self.out.write(b"\tstrd r0, r1, [r4]\n");
                self.out.write(b"\tadds r4, r4, #8\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Drop { .. } => {
                self.out.write(b"\tsubs r4, r4, #8\n");
                Ok(true)
            }
            lir::OpKind::Swap { .. } => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.emit_push_r2r3();
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::AddI64 => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.out.write(b"\tadds r0, r0, r2\n\tadc r1, r1, r3\n");
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::SubI64 => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.out.write(b"\tsubs r0, r0, r2\n\tsbc r1, r1, r3\n");
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::MulI64 => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                self.out.write(b"\tmuls r0, r2, r0\n\teors r1, r1\n");
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::Cmp { kind, .. } => {
                self.emit_pop_two_r2r3();
                self.emit_pop_two_r0r1();
                match kind {
                    lir::CmpKind::Eq => {
                        self.out.write(b"\tcmp r0, r2\n");
                        // Only `cmpeq` is conditional here, so the IT block has
                        // a single slot (`it`, not `itt`) — the following `ite`
                        // starts a fresh block.
                        self.out.write(b"\tit eq\n\tcmpeq r1, r3\n");
                        self.out
                            .write(b"\tite eq\n\tmoveq r0, #1\n\tmovne r0, #0\n");
                    }
                    lir::CmpKind::Ne => {
                        self.out.write(b"\tcmp r0, r2\n");
                        self.out.write(b"\tit ne\n\tcmpne r1, r3\n");
                        self.out
                            .write(b"\tite ne\n\tmovne r0, #1\n\tmoveq r0, #0\n");
                    }
                    lir::CmpKind::Lt => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out
                            .write(b"\tite lt\n\tmovlt r0, #1\n\tmovge r0, #0\n");
                    }
                    lir::CmpKind::Le => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out
                            .write(b"\tite le\n\tmovle r0, #1\n\tmovgt r0, #0\n");
                    }
                    lir::CmpKind::Gt => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out
                            .write(b"\tite gt\n\tmovgt r0, #1\n\tmovle r0, #0\n");
                    }
                    lir::CmpKind::Ge => {
                        self.out.write(b"\tcmp r1, r3\n");
                        self.out.write(b"\tbne 2f\n\tcmp r0, r2\n2:\n");
                        self.out
                            .write(b"\tite ge\n\tmovge r0, #1\n\tmovlt r0, #0\n");
                    }
                }
                self.out.write(b"\teors r1, r1\n");
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::AndBool => {
                self.emit_pop_one_r2();
                self.emit_pop_one_r0();
                self.out.write(b"\tands r0, r0, r2\n");
                self.emit_bool_normalize();
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::OrBool => {
                self.emit_pop_one_r2();
                self.emit_pop_one_r0();
                self.out.write(b"\torrs r0, r0, r2\n");
                self.emit_bool_normalize();
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::NotBool => {
                self.emit_pop_two_r0r1();
                self.out.write(b"\tcmp r0, #0\n");
                // `ite` (2 slots): movne (T/ne), moveq (E/eq).  The `eors` that
                // clears the high word is unconditional and must stay OUTSIDE
                // the IT block — `itte` would wrongly pull it in.
                self.out
                    .write(b"\tite ne\n\tmovne r0, #0\n\tmoveq r0, #1\n");
                self.out.write(b"\teors r1, r1\n");
                self.emit_push_r0r1();
                Ok(true)
            }
            lir::OpKind::InterruptDisable => {
                self.out.write(b"\tcpsid i\n");
                Ok(true)
            }
            lir::OpKind::InterruptEnable => {
                self.out.write(b"\tcpsie i\n");
                Ok(true)
            }
            lir::OpKind::LocalSet { slot, .. } => {
                let offset = (slot as u32) * 8;
                self.out.write(b"\tsubs r4, r4, #8\n");
                self.out.write(b"\tldrd r0, r1, [r4]\n");
                self.out.write(b"\tstrd r0, r1, [sp, #");
                write_u32(self.out, offset);
                self.out.write(b"]\n");
                Ok(true)
            }
            lir::OpKind::LocalGet { slot, .. } => {
                let offset = (slot as u32) * 8;
                self.out.write(b"\tldrd r0, r1, [sp, #");
                write_u32(self.out, offset);
                self.out.write(b"]\n");
                self.emit_push_r0r1();
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Call { name, .. } => {
                self.out.write(b"\tbl ");
                write_sym_label(self.out, name.as_bytes());
                self.out.write(b"\n");
                Ok(true)
            }
            lir::OpKind::Br { target } => {
                self.out.write(b"\tb .b");
                write_u32(self.out, base);
                self.out.write(b"_");
                write_u32(self.out, target.0 as u32);
                self.out.write(b"\n");
                Ok(true)
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
                Ok(true)
            }
            lir::OpKind::Ret => {
                self.out.write(b"\tb .endword_");
                write_u32(self.out, base);
                self.out.write(b"\n");
                Ok(true)
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
                // Pop address (low word of i64, discard high word).
                self.emit_pop_one_r0();
                let (bits, signed) = prim_bits_signed(_w, ty)
                    .ok_or(CodegenError::UnsupportedOp { op_name: b"Load" })?;
                match (bits, signed) {
                    (8, true) => self.out.write(b"\tldrsb r0, [r0]\n"),
                    (8, false) => self.out.write(b"\tldrb r0, [r0]\n"),
                    (16, true) => self.out.write(b"\tldrsh r0, [r0]\n"),
                    (16, false) => self.out.write(b"\tldrh r0, [r0]\n"),
                    (32, _) => self.out.write(b"\tldr r0, [r0]\n"),
                    (64, _) => self.out.write(b"\tldrd r0, r1, [r0]\n"),
                    _ => return Err(CodegenError::UnsupportedOp { op_name: b"Load" }),
                }
                if bits < 64 {
                    // Zero/sign-extend high word for smaller types.
                    if signed {
                        // r0 is already sign-extended by ldrsb/ldrsh; for
                        // 32-bit signed we need to ASR r0 by 31 to get the
                        // sign bit into r1.
                        if bits <= 32 {
                            self.out.write(b"\tasrs r1, r0, #31\n");
                        }
                    } else {
                        self.out.write(b"\teors r1, r1\n");
                    }
                }
                self.emit_push_r0r1();
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Store { ty } => {
                // Pop value (i64 → r0:r1 low:high), then pop address (low word).
                self.emit_pop_two_r0r1();
                self.emit_pop_one_r2();
                let (bits, _signed) = prim_bits_signed(_w, ty)
                    .ok_or(CodegenError::UnsupportedOp { op_name: b"Store" })?;
                match bits {
                    8 => self.out.write(b"\tstrb r0, [r2]\n"),
                    16 => self.out.write(b"\tstrh r0, [r2]\n"),
                    32 => self.out.write(b"\tstr r0, [r2]\n"),
                    64 => self.out.write(b"\tstrd r0, r1, [r2]\n"),
                    _ => return Err(CodegenError::UnsupportedOp { op_name: b"Store" }),
                }
                Ok(true)
            }
            lir::OpKind::AddrOf {
                const_addr: Some(addr),
                ..
            } => {
                let _ = self.mode; // unused but proves we reach this arm
                let low = addr as u32;
                self.emit_const32(low);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.out.write(b"\teors r0, r0\n");
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::AddrOf {
                place,
                const_addr: None,
                ..
            } => {
                if find_resource_decl(self.module, self.src, place.as_bytes()).is_none() {
                    return Err(CodegenError::UnsupportedAddrOf);
                }
                self.out.write(b"\tldr r0, =");
                write_res_label(
                    self.out,
                    slice_span(self.src, self.module.name),
                    place.as_bytes(),
                );
                self.out.write(b"\n\teors r1, r1\n");
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.out.write(b"\tstr r1, [r4]\n\tadds r4, r4, #4\n");
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::PtrAddConst { offset, .. } => {
                // Pop pointer (low word, discard high).
                self.emit_pop_one_r0();
                if offset <= 255 {
                    self.out.write(b"\tadds r0, r0, #");
                    write_u32(self.out, offset);
                    self.out.write(b"\n");
                } else {
                    // Large offset: load into a temp register and add.
                    self.out.write(b"\tldr r1, =");
                    write_hex(self.out, offset as u64);
                    self.out.write(b"\n\tadds r0, r0, r1\n");
                }
                // Push as i64 (high word = 0, unsigned).
                self.out.write(b"\teors r1, r1\n");
                self.emit_push_r0r1();
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::PtrAddIndex { scale, .. } => {
                // Pop index (low word), pop base (low word), base += index*scale.
                self.emit_pop_one_r0();
                self.emit_pop_one_r2();
                if scale == 0 {
                    // index is irrelevant; base is the result.
                } else if scale.is_power_of_two() {
                    let shift = scale.trailing_zeros();
                    self.out.write(b"\tlsls r0, r0, #");
                    write_u32(self.out, shift);
                    self.out.write(b"\n\tadds r0, r2, r0\n");
                } else {
                    self.out.write(b"\tldr r1, =");
                    write_hex(self.out, scale as u64);
                    self.out.write(b"\n\tmuls r0, r1, r0\n");
                    self.out.write(b"\tadds r0, r2, r0\n");
                }
                // Push as i64 (high word = 0).
                self.out.write(b"\teors r1, r1\n");
                self.emit_push_r0r1();
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
                self.emit_pop_two_r0r1();
                // Mask/sign-extend from source width.
                if from_bits < 64 {
                    let mask = (1u64 << from_bits) - 1;
                    if from_signed {
                        let shift_amt = 32 - from_bits;
                        self.out.write(b"\tlsls r0, r0, #");
                        write_u32(self.out, shift_amt as u32);
                        self.out.write(b"\n\tasrs r0, r0, #");
                        write_u32(self.out, shift_amt as u32);
                        self.out.write(b"\n\tasrs r1, r0, #31\n");
                    } else {
                        if mask <= 0xFFFF {
                            self.out.write(b"\tands r0, r0, #");
                            write_hex(self.out, mask);
                            self.out.write(b"\n");
                        } else {
                            self.out.write(b"\tldr r1, =");
                            write_hex(self.out, mask);
                            self.out.write(b"\n\tands r0, r0, r1\n");
                        }
                        self.out.write(b"\teors r1, r1\n");
                    }
                }
                // Normalize to bool if target is bool.
                if to_prim == lir::Prim::Bool && from_prim != lir::Prim::Bool {
                    self.out.write(b"\tcmp r0, #0\n");
                    self.out
                        .write(b"\tite ne\n\tmovne r0, #1\n\tmoveq r0, #0\n");
                    self.out.write(b"\teors r1, r1\n");
                }
                // Mask/sign-extend to target width.
                if to_bits < 64 && to_prim != lir::Prim::Bool {
                    let mask = (1u64 << to_bits) - 1;
                    if to_signed {
                        let shift_amt = 32 - to_bits;
                        self.out.write(b"\tlsls r0, r0, #");
                        write_u32(self.out, shift_amt as u32);
                        self.out.write(b"\n\tasrs r0, r0, #");
                        write_u32(self.out, shift_amt as u32);
                        self.out.write(b"\n\tasrs r1, r0, #31\n");
                    } else {
                        if mask <= 0xFFFF {
                            self.out.write(b"\tands r0, r0, #");
                            write_hex(self.out, mask);
                            self.out.write(b"\n");
                        } else {
                            self.out.write(b"\tldr r1, =");
                            write_hex(self.out, mask);
                            self.out.write(b"\n\tands r0, r0, r1\n");
                        }
                        self.out.write(b"\teors r1, r1\n");
                    }
                }
                self.emit_push_r0r1();
                self.emit_ds_high_update();
                Ok(true)
            }
            lir::OpKind::Bitcast { .. } => {
                // On ARM all values are 8 bytes on the DS; bitcast
                // does not change the bit pattern, so it's a no-op.
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
            lir::OpKind::MmioVolLoad { ty, place: _ } => {
                // Load from the address that's already on the DS (pushed by
                // MmioPlace or AddrOf). Pop the address, load the value.
                let (bits, _signed) =
                    prim_bits_signed(_w, ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolLoad",
                    })?;
                self.emit_pop_one_r0();
                match bits {
                    8 => self.out.write(b"\tldrb r0, [r0]\n"),
                    16 => self.out.write(b"\tldrh r0, [r0]\n"),
                    32 => self.out.write(b"\tldr r0, [r0]\n"),
                    64 => self.out.write(b"\tldrd r0, r1, [r0]\n"),
                    _ => {
                        return Err(CodegenError::UnsupportedOp {
                            op_name: b"MmioVolLoad",
                        })
                    }
                }
                if bits < 64 {
                    self.out.write(b"\teors r1, r1\n");
                }
                self.emit_push_r0r1();
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::MmioVolStore {
                ty,
                place: _,
                access: _,
            } => {
                // Pop value (i64), pop address, store.
                let (bits, _signed) =
                    prim_bits_signed(_w, ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolStore",
                    })?;
                self.emit_pop_two_r0r1();
                self.emit_pop_one_r2();
                match bits {
                    8 => self.out.write(b"\tstrb r0, [r2]\n"),
                    16 => self.out.write(b"\tstrh r0, [r2]\n"),
                    32 => self.out.write(b"\tstr r0, [r2]\n"),
                    64 => self.out.write(b"\tstrd r0, r1, [r2]\n"),
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
                // Load register, extract field via mask+shift.
                let (rbits, _) =
                    prim_bits_signed(_w, reg_ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolLoadField",
                    })?;
                let (fbits, f_signed) =
                    prim_bits_signed(_w, field_ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolLoadField",
                    })?;
                self.emit_pop_one_r0();
                match rbits {
                    32 => self.out.write(b"\tldr r0, [r0]\n"),
                    64 => self.out.write(b"\tldrd r0, r1, [r0]\n"),
                    _ => {
                        return Err(CodegenError::UnsupportedOp {
                            op_name: b"MmioVolLoadField",
                        })
                    }
                }
                // Apply mask and shift
                if shift > 0 {
                    self.out.write(b"\tlsrs r0, r0, #");
                    write_u32(self.out, shift as u32);
                    self.out.write(b"\n");
                }
                if mask != 0 && mask != u64::MAX {
                    if mask <= 0xFFFF {
                        self.out.write(b"\tands r0, r0, #");
                        write_hex(self.out, mask);
                        self.out.write(b"\n");
                    } else {
                        self.out.write(b"\tldr r1, =");
                        write_hex(self.out, mask);
                        self.out.write(b"\n\tands r0, r0, r1\n");
                    }
                }
                if fbits < 64 {
                    if f_signed {
                        let sign_bit = (fbits - 1) as u32;
                        let shift_amt = 32 - sign_bit;
                        self.out.write(b"\tlsls r0, r0, #");
                        write_u32(self.out, shift_amt);
                        self.out.write(b"\n\tasrs r0, r0, #");
                        write_u32(self.out, shift_amt);
                        self.out.write(b"\n");
                        self.out.write(b"\tasrs r1, r0, #31\n");
                    } else {
                        self.out.write(b"\teors r1, r1\n");
                    }
                }
                self.emit_push_r0r1();
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
                // Pop value, pop addr, load-modify-write the register.
                // ARM MMIO registers are assumed 32-bit (the common case on
                // Cortex-M).  r0 = value low word, r1 = high (discarded).
                let (fbits, _) =
                    prim_bits_signed(_w, field_ty).ok_or(CodegenError::UnsupportedOp {
                        op_name: b"MmioVolStoreField",
                    })?;
                self.emit_pop_two_r0r1();
                let _ = fbits;
                self.emit_pop_one_r2(); // r2 = addr
                                        // Load current register value (32-bit).
                self.out.write(b"\tldr r3, [r2]\n");
                // Clear field bits: clear_mask = !(mask << shift) & 0xFFFFFFFF
                let shifted_mask = mask.wrapping_shl(shift as u32) & 0xFFFFFFFF;
                let clear = (!shifted_mask) & 0xFFFFFFFF;
                if clear <= 0xFFFF {
                    self.out.write(b"\tands r3, r3, #");
                    write_hex(self.out, clear);
                    self.out.write(b"\n");
                } else {
                    self.out.write(b"\tldr r1, =");
                    write_hex(self.out, clear);
                    self.out.write(b"\n\tands r3, r3, r1\n");
                }
                // Shift value to field position and OR (r1 is dead, reused as scratch).
                if shift > 0 {
                    self.out.write(b"\tlsls r0, r0, #");
                    write_u32(self.out, shift as u32);
                    self.out.write(b"\n");
                }
                self.out.write(b"\torrs r3, r3, r0\n");
                // Write back
                self.out.write(b"\tstr r3, [r2]\n");
                Ok(())
            }
            lir::OpKind::MmioPlace { addr, .. } => {
                // Push the MMIO address onto DS.
                let low = addr as u32;
                self.emit_const32(low);
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.out.write(b"\teors r0, r0\n");
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.emit_ds_high_update();
                Ok(())
            }
            lir::OpKind::TaskSpawn { name, .. } => {
                self.uses_tasks = true;
                // Load task body pointer into r0, call __task_spawn.
                self.out.write(b"\tldr r0, =");
                write_sym_label(self.out, name.as_bytes());
                self.out.write(b"\n\tbl __task_spawn\n");
                // Push returned task ID (r0) onto DS as i64 (high word = 0).
                self.out.write(b"\tstr r0, [r4]\n\tadds r4, r4, #4\n");
                self.out
                    .write(b"\teors r1, r1\n\tstr r1, [r4]\n\tadds r4, r4, #4\n");
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
                    self.out.write(b"\tsubs r4, r4, #8\n\tldr r0, [r4]\n");
                    self.out.write(b"\tstr r0, [sp, #");
                    write_u32(self.out, offset);
                    self.out.write(b"]\n");
                    // Store length at [sp+offset+4]
                    self.out.write(b"\tmovs r0, #");
                    write_u32(self.out, len);
                    self.out.write(b"\n\tstr r0, [sp, #");
                    write_u32(self.out, offset + 4);
                    self.out.write(b"]\n");
                    // Push address of slot as result pointer
                    self.out.write(b"\tadd r0, sp, #");
                    write_u32(self.out, offset);
                    self.out.write(b"\n");
                    self.out.write(b"\teors r1, r1\n");
                    self.emit_push_r0r1();
                    self.emit_ds_high_update();
                } else if ty_name == b"RegionRef" || ty_name == b"RegionRefMut" {
                    // For region refs, just dup the value on DS.
                    self.out.write(b"\tsubs r4, r4, #8\n");
                    self.out.write(b"\tldrd r0, r1, [r4]\n");
                    self.out.write(b"\tstrd r0, r1, [r4]\n");
                    self.out.write(b"\tadds r4, r4, #8\n");
                    self.out.write(b"\tstrd r0, r1, [r4]\n");
                    self.out.write(b"\tadds r4, r4, #8\n");
                    self.emit_ds_high_update();
                }
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
        self.out
            .write(b"\tite ne\n\tmovne r0, #1\n\tmoveq r0, #0\n");
    }

    // ---- Trap ----

    fn emit_trap_with_loc(&mut self, code: u32, span: Span) {
        if self.debug_trap_loc {
            // Register contract for __lang_trap_loc (ARM AAPCS):
            //   r0  = trap_code
            //   r1  = valid (1)
            //   r2  = source_line
            //   r3  = word_hash low 32 bits
            //   r12 = word_hash high 32 bits
            let (line, _col) = line_col(self.src, span.start);
            let wh = self.cur_word_id;
            let lo = wh as u32;
            let hi = (wh >> 32) as u32;

            emit_thumb_mov32(self.out, 0, code);
            self.out.write(b"\tmovs r1, #1\n");
            emit_thumb_mov32(self.out, 2, line);
            emit_thumb_mov32(self.out, 3, lo);
            emit_thumb_mov32(self.out, 12, hi);
            self.out.write(b"\tb __lang_trap_loc\n");
        } else {
            self.out.write(b"\tmovs r0, #");
            write_u32(self.out, code);
            self.out.write(b"\n\tb __lang_trap\n");
        }
    }

    // ---- String interning ----

    /// Intern a string span: return an existing ID or allocate a new one.
    /// Idempotent — identical spans yield the same `__lang_str_<id>`.
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
