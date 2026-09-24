use codegen_core::strings::STR_TABLE_CAP;
use codegen_core::{AsmMode, CodegenError};
use frontend::span::Span;
use ir as lir;

use crate::channel;
use crate::mmio;
use crate::ophelpers::{
    emit_binop, emit_cmp, emit_drop, emit_dup, emit_load_local, emit_push_i64, emit_push_rax,
    emit_push_u64, emit_store_local, emit_swap, write_label, write_u32, write_u64_hex,
};
use crate::region;
use crate::task;
use crate::util::{
    count_scoped_slices, find_resource_decl, fnv1a_u64, is_exported, line_col, locals_bytes_ir,
    mask_for_bits, max_local_slot_ir, prim_ty, prim_ty_bits_signed, slice_span, type_class,
    write_res_label,
};
use crate::X86_64HostedBackend;

impl<'a> X86_64HostedBackend<'a> {
    pub fn emit_word(&mut self, w: &lir::Word) -> Result<(), CodegenError> {
        self.cur_word_id = fnv1a_u64(w.name.as_bytes());
        // P6: fuse this word's aperture-use entries into the module table.
        for wu in w.apertures.iter() {
            codegen_core::merge_aperture_use(&mut self.mi_apertures, &mut self.mi_aperture_count, wu);
        }
        // Collect debug metadata for every word when debug_trap_loc is set.
        if self.debug_trap_loc {
            let idx = self.debug_word_count;
            if idx < self.debug_words.len() {
                if let Some(name) = lir::Atom::new(w.name.as_bytes()) {
                    self.debug_words[idx] = Some(crate::DebugWordInfo {
                        name,
                        net: w.bound.net,
                        high: w.bound.wire_u32(),
                        effects: w.performs.bits(),
                    });
                    self.debug_word_count = idx + 1;
                }
            }
        }
        self.out.write(b"\n");
        if self.mode == AsmMode::Object && is_exported(self.module, self.src, w.name.as_bytes()) {
            self.out.write(b"public ");
            write_label(self.out, w.name.as_bytes());
            self.out.write(b"\n");

            // Collect export metadata for .lang.modinfo (S2 Phase 1).
            let idx = self.mi_export_count;
            if idx < self.mi_exports.len() {
                let n = lir::Atom::new(w.name.as_bytes()).unwrap_or(lir::AT_EMPTY);
                self.mi_exports[idx] = crate::ModInfoExport {
                    name: n,
                    effects: w.performs.bits(),
                    requires_caps: w.requires.bits(),
                    stack_bound: w.bound.wire_u32(),
                };
                self.mi_export_count = idx + 1;
            } else {
                return Err(CodegenError::ModInfoTooLarge);
            }
        }
        write_label(self.out, w.name.as_bytes());
        self.out.write(b":\n");

        let base = self.fresh_label();

        let slots = max_local_slot_ir(w).map(|m| (m as u32) + 1).unwrap_or(0);
        let locals_bytes = locals_bytes_ir(slots);
        let scoped_slots = count_scoped_slices(w);
        let mut frame_bytes = locals_bytes + (scoped_slots * 16);
        if !frame_bytes.is_multiple_of(16) {
            frame_bytes += 8;
        }
        self.scoped_base = locals_bytes;
        self.scoped_slots = scoped_slots;
        self.scoped_next = 0;
        if frame_bytes > 0 {
            self.out.write(b"  sub rsp, ");
            write_u32(self.out, frame_bytes);
            self.out.write(b"\n");
        }

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

    fn emit_stack_control_ops(
        &mut self,
        w: &lir::Word,
        op: &lir::Op,
        _base: u32,
    ) -> Result<bool, CodegenError> {
        match op.kind {
            lir::OpKind::ConstI64(v) => {
                emit_push_i64(self.out, v);
                Ok(true)
            }
            lir::OpKind::ConstBool(v) => {
                emit_push_i64(self.out, if v { 1 } else { 0 });
                Ok(true)
            }
            lir::OpKind::ConstStr(span) => {
                let id = self.intern_str(span)?;
                self.out.write(b"  mov rax, __lang_str_");
                write_u32(self.out, id);
                self.out.write(b"\n");
                emit_push_rax(self.out);
                Ok(true)
            }

            lir::OpKind::AddrOf {
                base: lir::AddrOfBase::Mmio { aperture, offset },
                ..
            } => {
                self.uses_mmio = true;
                let addr = self.mmio_aperture_addr(aperture, offset)?;
                emit_push_u64(self.out, addr);
                Ok(true)
            }
            lir::OpKind::AddrOf {
                base: lir::AddrOfBase::Runtime,
                place,
                mutable: _,
            } => {
                // Resources live at a runtime symbol; emit its address so
                // `lock [ &!Res … ]` works on the hosted target (BUG-004).
                if find_resource_decl(self.module, self.src, place.as_bytes()).is_none() {
                    return Err(CodegenError::UnsupportedAddrOf);
                }
                self.uses_resources = true;
                self.out.write(b"  mov rax, ");
                write_res_label(
                    self.out,
                    slice_span(self.src, self.module.name),
                    place.as_bytes(),
                );
                self.out.write(b"\n");
                emit_push_rax(self.out);
                Ok(true)
            }
            lir::OpKind::MmioPlace { aperture, offset, .. } => {
                self.uses_mmio = true;
                let addr = self.mmio_aperture_addr(aperture, offset)?;
                emit_push_u64(self.out, addr);
                Ok(true)
            }
            lir::OpKind::PtrAddConst { offset, .. } => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  add rax, ");
                write_u32(self.out, offset);
                self.out.write(b"\n");
                emit_push_rax(self.out);
                Ok(true)
            }
            lir::OpKind::PtrAddIndex { scale, .. } => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rcx, [r15]\n"); // idx
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n"); // base
                self.out.write(b"  imul rcx, ");
                write_u32(self.out, scale);
                self.out.write(b"\n");
                self.out.write(b"  add rax, rcx\n");
                emit_push_rax(self.out);
                Ok(true)
            }

            lir::OpKind::ScopedEnter { ty, len } => {
                match type_class(w, ty) {
                    lir::TypeClass::Slice => {
                        if self.scoped_next >= self.scoped_slots {
                            return Err(CodegenError::ScopedAllocationOverflow);
                        }
                        let slot = self.scoped_next;
                        self.scoped_next = self.scoped_next.wrapping_add(1);
                        let offset = self.scoped_base + (slot * 16);
                        self.out.write(b"  mov rax, [r15-8]\n");
                        self.out.write(b"  mov [rsp+");
                        write_u32(self.out, offset);
                        self.out.write(b"], rax\n");
                        self.out.write(b"  mov qword [rsp+");
                        write_u32(self.out, offset + 8);
                        self.out.write(b"], ");
                        write_u64_hex(self.out, len as u64);
                        self.out.write(b"\n");
                        self.out.write(b"  lea rax, [rsp+");
                        write_u32(self.out, offset);
                        self.out.write(b"]\n");
                        emit_push_rax(self.out);
                        return Ok(true);
                    }
                    lir::TypeClass::RegionRef | lir::TypeClass::RegionRefMut => {
                        emit_dup(self.out);
                        return Ok(true);
                    }
                    // D-13: an unmatched class is a loud error, never a
                    // silent no-op (the G-5 bug class this slice retires).
                    _ => {
                        return Err(CodegenError::UnsupportedOp {
                            op_name: b"ScopedEnter",
                        });
                    }
                }
            }
            lir::OpKind::TaskSpawn { name, .. } => {
                self.uses_tasks = true;
                task::emit_task_spawn(self, name.as_bytes());
                Ok(true)
            }

            lir::OpKind::Dup { .. } => {
                emit_dup(self.out);
                Ok(true)
            }
            lir::OpKind::Drop { .. } => {
                emit_drop(self.out);
                Ok(true)
            }
            lir::OpKind::Swap { .. } => {
                emit_swap(self.out);
                Ok(true)
            }

            lir::OpKind::AddI64 => {
                emit_binop(self.out, b"add");
                Ok(true)
            }
            lir::OpKind::SubI64 => {
                emit_binop(self.out, b"sub");
                Ok(true)
            }
            lir::OpKind::MulI64 => {
                emit_binop(self.out, b"imul");
                Ok(true)
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
                Ok(true)
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
                Ok(true)
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
                Ok(true)
            }
            lir::OpKind::NotBool => {
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                self.out.write(b"  cmp rax, 0\n");
                self.out.write(b"  sete al\n");
                self.out.write(b"  movzx rax, al\n");
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(true)
            }
            lir::OpKind::InterruptDisable => {
                self.out.write(b"  cli\n");
                Ok(true)
            }
            lir::OpKind::InterruptEnable => {
                self.out.write(b"  sti\n");
                Ok(true)
            }

            lir::OpKind::LocalSet { slot, .. } => {
                emit_store_local(self.out, slot as u32);
                Ok(true)
            }
            lir::OpKind::LocalGet { slot, .. } => {
                emit_load_local(self.out, slot as u32);
                Ok(true)
            }

            lir::OpKind::Cast { from, to } => {
                self.emit_cast(w, from, to);
                Ok(true)
            }
            lir::OpKind::Bitcast { .. } => Ok(true),

            _ => Ok(false),
        }
    }

    fn emit_memory_ops(
        &mut self,
        w: &lir::Word,
        op: &lir::Op,
        _base: u32,
    ) -> Result<bool, CodegenError> {
        match op.kind {
            lir::OpKind::Call { name, sig, .. } => {
                let n = name.as_bytes();
                if n == b"platform.io.log" {
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rcx, [r15]\n");
                    self.out.write(b"  mov rsi, [rcx]\n");
                    self.out.write(b"  mov rdx, [rcx+8]\n");
                    self.out.write(b"  mov rdi, 2\n");
                    self.out.write(b"  mov rax, 1\n");
                    self.out.write(b"  syscall\n");
                    return Ok(true);
                }
                if n == b"platform.channel.make" {
                    self.uses_channels = true;
                    self.uses_tasks = true;
                    channel::emit_chan_make(self);
                    return Ok(true);
                }
                if n == b"platform.channel.send" {
                    self.uses_channels = true;
                    self.uses_tasks = true;
                    channel::emit_chan_send(self, w, op, &sig);
                    return Ok(true);
                }
                if n == b"platform.channel.recv" {
                    self.uses_channels = true;
                    self.uses_tasks = true;
                    channel::emit_chan_recv(self, w, op, &sig);
                    return Ok(true);
                }
                if n == b"platform.mem.region-create" {
                    self.uses_regions = true;
                    region::emit_region_create(self);
                    return Ok(true);
                }
                if n == b"platform.mem.region-alloc" {
                    self.uses_regions = true;
                    region::emit_region_alloc(self);
                    return Ok(true);
                }
                if n == b"platform.mem.region-reset" {
                    self.uses_regions = true;
                    region::emit_region_reset(self);
                    return Ok(true);
                }
                if n == b"platform.mem.region-destroy" {
                    self.uses_regions = true;
                    region::emit_region_destroy(self);
                    return Ok(true);
                }
                if n == b"platform.time.now_ms" {
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
                    return Ok(true);
                }
                if n == b"platform.task.yield" {
                    self.uses_tasks = true;
                    task::emit_task_yield(self);
                    return Ok(true);
                }
                if n == b"platform.task.join" {
                    self.uses_tasks = true;
                    task::emit_task_join(self);
                    return Ok(true);
                }
                if n == b"platform.task.sleep-ms" {
                    self.uses_tasks = true;
                    task::emit_task_sleep_ms(self);
                    return Ok(true);
                }
                if n == b"platform.task.sleep-us" {
                    self.uses_tasks = true;
                    task::emit_task_sleep_us(self);
                    return Ok(true);
                }
                if n == b"platform.critical.enter" || n == b"platform.critical.exit" {
                    return Ok(true);
                }
                self.out.write(b"  call ");
                write_label(self.out, name.as_bytes());
                self.out.write(b"\n");
                Ok(true)
            }

            _ => Ok(false),
        }
    }

    fn emit_platform_ops(
        &mut self,
        w: &lir::Word,
        op: &lir::Op,
        base: u32,
    ) -> Result<(), CodegenError> {
        match op.kind {
            lir::OpKind::Load { ty } => {
                let (bits, signed) = prim_ty_bits_signed(w, ty)
                    .ok_or(CodegenError::UnknownTypeProperties { type_id: ty })?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                match (width, signed) {
                    (1, true) => self.out.write(b"  movsx rax, byte [rax]\n"),
                    (1, false) => self.out.write(b"  movzx rax, byte [rax]\n"),
                    (2, true) => self.out.write(b"  movsx rax, word [rax]\n"),
                    (2, false) => self.out.write(b"  movzx rax, word [rax]\n"),
                    (4, true) => self.out.write(b"  movsxd rax, dword [rax]\n"),
                    (4, false) => self.out.write(b"  mov eax, dword [rax]\n"),
                    (8, _) => self.out.write(b"  mov rax, qword [rax]\n"),
                    _ => return Err(CodegenError::UnknownTypeProperties { type_id: ty }),
                }
                self.out.write(b"  mov [r15], rax\n");
                self.out.write(b"  add r15, 8\n");
                Ok(())
            }
            lir::OpKind::Store { ty } => {
                let (bits, _signed) = prim_ty_bits_signed(w, ty)
                    .ok_or(CodegenError::UnknownTypeProperties { type_id: ty })?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rcx, [r15]\n");
                self.out.write(b"  sub r15, 8\n");
                self.out.write(b"  mov rax, [r15]\n");
                match width {
                    1 => self.out.write(b"  mov byte [rax], cl\n"),
                    2 => self.out.write(b"  mov word [rax], cx\n"),
                    4 => self.out.write(b"  mov dword [rax], ecx\n"),
                    8 => self.out.write(b"  mov qword [rax], rcx\n"),
                    _ => return Err(CodegenError::UnknownTypeProperties { type_id: ty }),
                }
                Ok(())
            }

            lir::OpKind::MmioVolLoad { ty, .. } => {
                self.uses_mmio = true;
                let (bits, signed) = prim_ty_bits_signed(w, ty)
                    .ok_or(CodegenError::UnknownTypeProperties { type_id: ty })?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                mmio::emit_mmio_load(self, width, signed, op.span)?;
                Ok(())
            }
            lir::OpKind::MmioVolStore { ty, write_kind, .. } => {
                self.uses_mmio = true;
                let (bits, _signed) = prim_ty_bits_signed(w, ty)
                    .ok_or(CodegenError::UnknownTypeProperties { type_id: ty })?;
                let width = core::cmp::max(1u32, (bits as u32) / 8);
                mmio::emit_mmio_store(self, width, write_kind, op.span)?;
                Ok(())
            }
            lir::OpKind::MmioVolLoadField {
                reg_ty,
                field_ty,
                mask,
                shift,
                ..
            } => {
                self.uses_mmio = true;
                let (reg_bits, _reg_signed) = prim_ty_bits_signed(w, reg_ty)
                    .ok_or(CodegenError::UnknownTypeProperties { type_id: reg_ty })?;
                let reg_width = core::cmp::max(1u32, (reg_bits as u32) / 8);
                let (field_bits, field_signed) = prim_ty_bits_signed(w, field_ty)
                    .ok_or(CodegenError::UnknownTypeProperties { type_id: field_ty })?;
                mmio::emit_mmio_load_field(
                    self,
                    reg_width,
                    field_bits,
                    field_signed,
                    mask,
                    shift,
                    op.span,
                )?;
                Ok(())
            }
            lir::OpKind::MmioVolStoreField {
                reg_ty,
                mask,
                shift,
                ..
            } => {
                self.uses_mmio = true;
                let (reg_bits, _reg_signed) = prim_ty_bits_signed(w, reg_ty)
                    .ok_or(CodegenError::UnknownTypeProperties { type_id: reg_ty })?;
                let reg_width = core::cmp::max(1u32, (reg_bits as u32) / 8);
                mmio::emit_mmio_store_field(self, reg_width, mask, shift, op.span)?;
                Ok(())
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
            _ => Err(CodegenError::UnsupportedOp {
                op_name: b"emit_op",
            }),
        }
    }

    fn emit_op(&mut self, w: &lir::Word, op: &lir::Op, base: u32) -> Result<(), CodegenError> {
        if self.emit_stack_control_ops(w, op, base)? {
            return Ok(());
        }
        if self.emit_memory_ops(w, op, base)? {
            return Ok(());
        }
        self.emit_platform_ops(w, op, base)
    }

    fn emit_cast(&mut self, w: &lir::Word, from: lir::TypeId, to: lir::TypeId) {
        let Some(from_prim) = prim_ty(w, from) else {
            return;
        };
        let Some(to_prim) = prim_ty(w, to) else {
            return;
        };
        if from_prim == to_prim {
            return;
        }
        let (from_bits, from_signed) = from_prim.bits_signed(64);
        let (to_bits, to_signed) = to_prim.bits_signed(64);

        self.out.write(b"  mov rax, [r15-8]\n");

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
        }

        if to_prim == lir::Prim::Bool && from_prim != lir::Prim::Bool {
            self.out.write(b"  cmp rax, 0\n");
            self.out.write(b"  setne al\n");
            self.out.write(b"  movzx rax, al\n");
        }

        self.out.write(b"  mov [r15-8], rax\n");
    }

    pub(crate) fn emit_trap_with_loc(&mut self, code: u32, span: Span) {
        if self.debug_trap_loc {
            // Register contract for __lang_trap_loc (x86_64 SysV ABI):
            //   rdi = trap_code (u32, zero-extended to 64 bits)
            //   rsi = valid flag  (1 = language trap, 0 = hardware fault)
            //   rdx = source line (u32, zero-extended)
            //   rcx = word_hash   (full fnv1a_u64, 64-bit)
            //
            // The host diagnostic decoder matches word_hash against the
            // modinfo sym_hash (abi-contract §6) to name the trapping word.
            let (line, _col) = line_col(self.src, span.start);
            self.out.write(b"  mov rdi, ");
            write_u32(self.out, code);
            self.out.write(b"\n");
            self.out.write(b"  mov rsi, 1\n");
            self.out.write(b"  mov rdx, ");
            write_u32(self.out, line);
            self.out.write(b"\n");
            self.out.write(b"  mov rcx, ");
            write_u64_hex(self.out, self.cur_word_id);
            self.out.write(b"\n");
            self.out.write(b"  jmp __lang_trap_loc\n");
        } else {
            self.out.write(b"  mov rdi, ");
            write_u32(self.out, code);
            self.out.write(b"\n");
            self.out.write(b"  jmp __lang_trap\n");
        }
    }

    fn intern_str(&mut self, span: Span) -> Result<u32, CodegenError> {
        for i in 0..self.str_len {
            if crate::util::slice_span(self.src, self.str_spans[i])
                == crate::util::slice_span(self.src, span)
            {
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
