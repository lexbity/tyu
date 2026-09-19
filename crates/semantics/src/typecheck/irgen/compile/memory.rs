use super::borrow::{mint_id, scan_conflict};
use super::*;
use crate::typecheck::mmio::MmioAccessMeta;
use crate::typecheck::place::parse_place_path;
use crate::typecheck::value::PLACE_NONE;

/// R2 (D-3): reject an access wider than the register's `atomic_max` (E3643).
fn check_atomic_width(
    reg_ty: crate::types::TypeAtom,
    meta: MmioAccessMeta,
    span: Span,
) -> Result<(), TcError> {
    // atomic_max is expressed in bits; the register width in bytes.
    let width_bits = mmio_type_width_bytes(reg_ty.as_bytes()).unwrap_or(1) as u32 * 8;
    if width_bits > meta.atomic_max as u32 {
        return Err(TcError::MmioOverWideAccess { span });
    }
    Ok(())
}

/// R1+R2 for whole-register stores (E3642/E3643).
fn check_store_semantics(
    reg_ty: crate::types::TypeAtom,
    meta: MmioAccessMeta,
    span: Span,
) -> Result<(), TcError> {
    // R1: a w1s/w1c store lowers to a read-modify-write; on an effectful
    // register the RMW's read is a phantom bus read.
    if meta.read_kind == ir::ReadKind::Effectful && meta.write_kind != ir::WriteKind::Plain {
        return Err(TcError::MmioPhantomRead { span });
    }
    check_atomic_width(reg_ty, meta, span)
}

impl<'a, 'r> IrWordGen<'a, 'r> {
    #[allow(dead_code)]
    pub(super) fn compile_field_access(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
        lex: &mut Lexer,
    ) -> Result<lir::BlockId, TcError> {
        let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
        let field_tok = lex.next();
        if field_tok.kind != TokenKind::Ident {
            return Err(TcError::FieldNotFound { span: op_span });
        }
        let field_atom = TypeAtom::new(&slice[field_tok.span.start..field_tok.span.end])
            .ok_or(TcError::FieldNotFound { span: op_span })?;
        let base = *stack
            .get(*sp - 1)
            .ok_or(TcError::StackUnderflow { span: op_span })?;
        let (struct_ty, mutable) = match base {
            Value::Ptr { ty, mutable, .. } => (ty, mutable),
            _ => return Err(TcError::FieldNotFound { span: op_span }),
        };
        let Some(sinfo) = self.nominals.structs.iter().find(|s| s.name == struct_ty) else {
            return Err(TcError::FieldNotFound { span: op_span });
        };
        let mut offset: u32 = 0;
        let mut found: Option<TypeAtom> = None;
        for field in sinfo.fields.iter() {
            let fsize = type_size_bytes(field.ty, self.nominals)
                .ok_or(TcError::FieldSizeError { span: op_span })?;
            let falign = field_align(fsize);
            offset = align_up(offset, falign);
            if field.name == field_atom {
                found = Some(field.ty);
                break;
            }
            offset = offset
                .checked_add(fsize)
                .ok_or(TcError::FieldSizeError { span: op_span })?;
        }
        let Some(field_ty) = found else {
            return Err(TcError::FieldNotFound { span: op_span });
        };
        let base_tid = if mutable {
            lir::TY_PTR_MUT
        } else {
            lir::TY_PTR
        };
        self.emit_op(
            cur,
            lir::OpKind::PtrAddConst {
                ty: base_tid,
                offset,
            },
            op_span,
        )?;
        let base_place = match stack[*sp - 1] {
            Value::Ptr { place, .. } => place,
            _ => PLACE_NONE,
        };
        stack[*sp - 1] = Value::Ptr {
            ty: field_ty,
            mutable,
            place: base_place,
        };
        Ok(cur)
    }

    pub(super) fn compile_index(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
        lex: &mut Lexer,
        allow_locals: bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
        let mut const_idx: Option<u32> = None;
        let mut is_dynamic = false;

        let next = lex.next();
        match next.kind {
            TokenKind::Number => {
                const_idx = parse_u32_any(&slice[next.span.start..next.span.end]);
                if const_idx.is_none() {
                    return Err(TcError::IndexError { span: op_span });
                }
            }
            TokenKind::PunctLParen => {
                let par = capture_balanced(
                    lex,
                    slice,
                    TokenKind::PunctLParen,
                    TokenKind::PunctRParen,
                    next.span.start,
                )
                .map_err(|_| TcError::IndexError { span: op_span })?;
                let inner = Span::new(span.start + par.start + 1, span.start + par.end - 1);
                cur = self.compile_span(cur, stack, sp, inner, allow_locals, observer)?;
                is_dynamic = true;
            }
            _ => {
                return Err(TcError::IndexError { span: op_span });
            }
        }

        if is_dynamic {
            if *sp == 0 {
                return Err(TcError::IndexError { span: op_span });
            }
            if stack[*sp - 1] != Value::Plain(TypeAtom::I64) {
                return Err(TcError::IndexError { span: op_span });
            }
        }

        let base_pos = if is_dynamic { *sp - 2 } else { *sp - 1 };
        if base_pos >= *sp {
            return Err(TcError::IndexError { span: op_span });
        }

        let base = stack[base_pos];
        let (elem_ty, scale, out_kind, base_tid) = match base {
            Value::Plain(t) => {
                let Some(elem) = array_elem_type(t) else {
                    return Err(TcError::IndexError { span: op_span });
                };
                let len = array_len(t).ok_or(TcError::IndexError { span: op_span })?;
                if let Some(idx) = const_idx {
                    if idx >= len {
                        return Err(TcError::ArrayIndexOob { span: op_span });
                    }
                }
                let size = type_size_bytes(elem, self.nominals)
                    .ok_or(TcError::IndexError { span: op_span })?;
                (elem, size, IndexOut::Value, lir::TY_PTR)
            }
            Value::Ptr { ty, mutable, .. } => {
                let Some(elem) = array_elem_type(ty) else {
                    return Err(TcError::IndexError { span: op_span });
                };
                let len = array_len(ty).ok_or(TcError::IndexError { span: op_span })?;
                if let Some(idx) = const_idx {
                    if idx >= len {
                        return Err(TcError::ArrayIndexOob { span: op_span });
                    }
                }
                let size = type_size_bytes(elem, self.nominals)
                    .ok_or(TcError::IndexError { span: op_span })?;
                let tid = if mutable {
                    lir::TY_PTR_MUT
                } else {
                    lir::TY_PTR
                };
                (elem, size, IndexOut::Ptr(mutable), tid)
            }
            Value::MmioPlace(MmioResolved::Reg(reg)) => {
                let Some(width) = mmio_type_width_bytes(reg.reg_ty.as_bytes()) else {
                    return Err(TcError::IndexError { span: op_span });
                };
                if let Some(idx) = const_idx {
                    if let Some(len) = reg.array_len {
                        if idx >= len {
                            return Err(TcError::MmioArrayIndexOob { span: op_span });
                        }
                    }
                }
                (reg.reg_ty, width, IndexOut::Mmio, lir::TY_MMIO)
            }
            Value::MmioPlace(MmioResolved::Field(_)) => {
                return Err(TcError::IndexError { span: op_span });
            }
            _ => return Err(TcError::IndexError { span: op_span }),
        };

        let base_place = match stack[base_pos] {
            Value::Ptr { place, .. } => place,
            Value::MmioPlace(_) => PLACE_NONE,
            _ => PLACE_NONE,
        };
        match out_kind {
            IndexOut::Value => {
                stack[base_pos] = Value::Ptr {
                    ty: elem_ty,
                    mutable: false,
                    place: base_place,
                };
            }
            IndexOut::Ptr(mutable) => {
                stack[base_pos] = Value::Ptr {
                    ty: elem_ty,
                    mutable,
                    place: base_place,
                };
            }
            IndexOut::Mmio => {}
        }

        if is_dynamic {
            self.emit_op(
                cur,
                lir::OpKind::PtrAddIndex {
                    ty: base_tid,
                    scale,
                },
                op_span,
            )?;
            *sp = (*sp).saturating_sub(1);
        } else {
            let offset = const_idx
                .expect("!is_dynamic => const_idx is Some")
                .saturating_mul(scale);
            self.emit_op(
                cur,
                lir::OpKind::PtrAddConst {
                    ty: base_tid,
                    offset,
                },
                op_span,
            )?;
        }

        if matches!(out_kind, IndexOut::Value) {
            let tid = self.ty_id_of_type(elem_ty, op_span)?;
            self.emit_op(cur, lir::OpKind::Load { ty: tid }, op_span)?;
            stack[base_pos] = Value::Plain(elem_ty);
        }
        Ok(cur)
    }

    pub(super) fn compile_addr_of(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
        lex: &mut Lexer,
    ) -> Result<lir::BlockId, TcError> {
        let mut_tok = tok.kind == TokenKind::PunctAmpBang;
        let place_path = parse_place_path(lex, slice).map_err(|_| TcError::PlaceParseFailed {
            span: Span::new(span.start + tok.span.start, span.start + tok.span.end),
        })?;
        // Adjust spans from slice-relative to self.src-absolute for resolve_mmio_place.
        let base = span.start;
        let mut abs_path = place_path;
        abs_path.root = Span::new(base + abs_path.root.start, base + abs_path.root.end);
        abs_path.full = Span::new(base + abs_path.full.start, base + abs_path.full.end);
        let place_abs = abs_path.full;
        let root_atom = TypeAtom::new(&self.src[abs_path.root.start..abs_path.root.end])
            .unwrap_or(TypeAtom::EMPTY);
        let mut addr_of_base = lir::AddrOfBase::Runtime;

        if let Some(res) = resolve_mmio_place(self.mmio, self.src, &abs_path, place_abs)? {
            match res {
                MmioResolved::Reg(reg) => {
                    if mut_tok && !access_can_write(reg.access) {
                        return Err(TcError::MmioAccessViolation { span: place_abs });
                    }
                    self.record_aperture(reg.aperture, reg.access, place_abs)?;
                    addr_of_base = lir::AddrOfBase::Mmio {
                        aperture: reg.aperture,
                        offset: reg.offset,
                    };
                    push(
                        stack,
                        sp,
                        Value::MmioPtr {
                            reg,
                            mutable: mut_tok,
                        },
                    )?;
                }
                MmioResolved::Field(_) => {
                    return Err(TcError::MmioFieldNotAddressable { span: place_abs })
                }
            }
        } else {
            if resource_ty(self.resources, root_atom).is_some() {
                let locked_res = self.ctx.lock_frame().and_then(|f| f.resource());
                if locked_res != Some(root_atom) {
                    if resource_is_isr_reachable(self.resources, root_atom) {
                        return Err(TcError::ResourceSharedUnlocked { span: place_abs });
                    }
                    return Err(TcError::CapMissing { span: place_abs });
                }
            } else if mut_tok {
                let root = root_atom;
                if find_local(&self.locals, self.local_len, root).is_some() {
                    return Err(TcError::MutRefToLocal {
                        span: abs_path.root_abs(0),
                    });
                }
            }
            if let Some(pointee) = self.resolve_place_pointee_ty(&abs_path, place_abs)? {
                // S-3: mint a borrow ledger entry; scan for conflicts.
                scan_conflict(
                    stack,
                    *sp,
                    &self.ledger,
                    self.ledger_len,
                    &self.local_place,
                    &self.local_live,
                    &self.local_tys,
                    self.local_len,
                    root_atom,
                    mut_tok,
                    place_abs,
                )?;
                let place = mint_id(
                    &mut self.ledger,
                    &mut self.ledger_len,
                    root_atom,
                    pointee,
                    place_abs,
                )?;
                push(
                    stack,
                    sp,
                    Value::Ptr {
                        ty: pointee,
                        mutable: mut_tok,
                        place,
                    },
                )?;
            } else {
                return Err(TcError::PlaceUnknownRoot { span: place_abs });
            }
        }

        let place_bytes = &self.src[abs_path.full.start..abs_path.full.end];
        self.emit_op(
            cur,
            lir::OpKind::AddrOf {
                place: lir_atom(place_bytes)?,
                mutable: mut_tok,
                base: addr_of_base,
            },
            place_abs,
        )?;
        Ok(cur)
    }

    /// S-14: unified `.` operator — handles field access, static index, and
    /// dynamic index with auto-projection (value-extract vs pointer-address).
    pub(super) fn compile_dot_op(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
        lex: &mut Lexer,
    ) -> Result<lir::BlockId, TcError> {
        let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
        let next = lex.next();
        match next.kind {
            TokenKind::Ident => {
                // .field_name — field access (old `->field` or `.field`)
                let field_atom = TypeAtom::new(&slice[next.span.start..next.span.end])
                    .ok_or(TcError::FieldNotFound { span: op_span })?;
                let base = *stack
                    .get(*sp - 1)
                    .ok_or(TcError::StackUnderflow { span: op_span })?;
                match base {
                    Value::Ptr { ty, mutable, place } => {
                        // Auto-projection: `.x` on a Ptr → field address (old -> behavior).
                        let Some(sinfo) = self.nominals.structs.iter().find(|s| s.name == ty)
                        else {
                            return Err(TcError::FieldNotFound { span: op_span });
                        };
                        let mut offset: u32 = 0;
                        let mut found: Option<TypeAtom> = None;
                        for field in sinfo.fields.iter() {
                            let fsize = type_size_bytes(field.ty, self.nominals)
                                .ok_or(TcError::FieldSizeError { span: op_span })?;
                            let falign = field_align(fsize);
                            offset = align_up(offset, falign);
                            if field.name == field_atom {
                                found = Some(field.ty);
                                break;
                            }
                            offset = offset
                                .checked_add(fsize)
                                .ok_or(TcError::FieldSizeError { span: op_span })?;
                        }
                        let Some(field_ty) = found else {
                            return Err(TcError::FieldNotFound { span: op_span });
                        };
                        let base_tid = if mutable {
                            lir::TY_PTR_MUT
                        } else {
                            lir::TY_PTR
                        };
                        self.emit_op(
                            cur,
                            lir::OpKind::PtrAddConst {
                                ty: base_tid,
                                offset,
                            },
                            op_span,
                        )?;
                        stack[*sp - 1] = Value::Ptr {
                            ty: field_ty,
                            mutable,
                            place,
                        };
                        Ok(cur)
                    }
                    Value::Plain(struct_ty) => {
                        // Auto-projection: `.x` on a value → field extraction.
                        let Some(sinfo) =
                            self.nominals.structs.iter().find(|s| s.name == struct_ty)
                        else {
                            return Err(TcError::FieldNotFound { span: op_span });
                        };
                        let mut offset: u32 = 0;
                        let mut found: Option<TypeAtom> = None;
                        for field in sinfo.fields.iter() {
                            let fsize = type_size_bytes(field.ty, self.nominals)
                                .ok_or(TcError::FieldSizeError { span: op_span })?;
                            let falign = field_align(fsize);
                            offset = align_up(offset, falign);
                            if field.name == field_atom {
                                found = Some(field.ty);
                                break;
                            }
                            offset = offset
                                .checked_add(fsize)
                                .ok_or(TcError::FieldSizeError { span: op_span })?;
                        }
                        let Some(field_ty) = found else {
                            return Err(TcError::FieldNotFound { span: op_span });
                        };
                        let struct_tid = self.ty_id_of_type(struct_ty, op_span)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::PtrAddConst {
                                ty: struct_tid,
                                offset,
                            },
                            op_span,
                        )?;
                        let field_tid = self.ty_id_of_type(field_ty, op_span)?;
                        self.emit_op(cur, lir::OpKind::Load { ty: field_tid }, op_span)?;
                        let _ = pop(stack, sp);
                        push(stack, sp, Value::Plain(field_ty))?;
                        Ok(cur)
                    }
                    _ => Err(TcError::FieldNotFound { span: op_span }),
                }
            }
            TokenKind::Number => {
                // .N — static index (old 'N behavior)
                let const_idx =
                    crate::typecheck::util::parse_u32_any(&slice[next.span.start..next.span.end])
                        .ok_or(TcError::IndexError { span: op_span })?;
                let base_pos = *sp - 1;
                let base = stack[base_pos];
                let (elem_ty, scale, out_kind, base_tid) = match base {
                    Value::Plain(t) => {
                        let Some(elem) = array_elem_type(t) else {
                            return Err(TcError::IndexError { span: op_span });
                        };
                        let len = array_len(t).ok_or(TcError::IndexError { span: op_span })?;
                        if const_idx >= len {
                            return Err(TcError::ArrayIndexOob { span: op_span });
                        }
                        let size = type_size_bytes(elem, self.nominals)
                            .ok_or(TcError::IndexError { span: op_span })?;
                        (elem, size, IndexOut::Value, lir::TY_PTR)
                    }
                    Value::Ptr { ty, mutable, .. } => {
                        let Some(elem) = array_elem_type(ty) else {
                            return Err(TcError::IndexError { span: op_span });
                        };
                        let len = array_len(ty).ok_or(TcError::IndexError { span: op_span })?;
                        if const_idx >= len {
                            return Err(TcError::ArrayIndexOob { span: op_span });
                        }
                        let size = type_size_bytes(elem, self.nominals)
                            .ok_or(TcError::IndexError { span: op_span })?;
                        let tid = if mutable {
                            lir::TY_PTR_MUT
                        } else {
                            lir::TY_PTR
                        };
                        (elem, size, IndexOut::Ptr(mutable), tid)
                    }
                    Value::MmioPlace(MmioResolved::Reg(reg)) => {
                        let Some(width) = mmio_type_width_bytes(reg.reg_ty.as_bytes()) else {
                            return Err(TcError::IndexError { span: op_span });
                        };
                        if let Some(len) = reg.array_len {
                            if const_idx >= len {
                                return Err(TcError::MmioArrayIndexOob { span: op_span });
                            }
                        }
                        (reg.reg_ty, width, IndexOut::Mmio, lir::TY_MMIO)
                    }
                    _ => return Err(TcError::IndexError { span: op_span }),
                };
                match out_kind {
                    IndexOut::Value => {
                        stack[base_pos] = Value::Ptr {
                            ty: elem_ty,
                            mutable: false,
                            place: PLACE_NONE,
                        };
                    }
                    IndexOut::Ptr(mutable) => {
                        let base_place = match stack[base_pos] {
                            Value::Ptr { place, .. } => place,
                            _ => PLACE_NONE,
                        };
                        stack[base_pos] = Value::Ptr {
                            ty: elem_ty,
                            mutable,
                            place: base_place,
                        };
                    }
                    IndexOut::Mmio => {}
                }
                let offset = const_idx.saturating_mul(scale);
                self.emit_op(
                    cur,
                    lir::OpKind::PtrAddConst {
                        ty: base_tid,
                        offset,
                    },
                    op_span,
                )?;
                if matches!(out_kind, IndexOut::Value) {
                    let tid = self.ty_id_of_type(elem_ty, op_span)?;
                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, op_span)?;
                    stack[base_pos] = Value::Plain(elem_ty);
                }
                Ok(cur)
            }
            _ => Err(TcError::IndexError { span: op_span }),
        }
    }

    pub(super) fn compile_load_store(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name_abs: Span,
        name: &[u8],
        _tok: Token,
    ) -> Result<lir::BlockId, TcError> {
        let is_load = name[0] == b'@';
        let typed = name.len() > 1;
        let ty_atom = if typed {
            Some(
                TypeAtom::new(&name[1..])
                    .ok_or(TcError::MmioTypedAtomInvalid { span: name_abs })?,
            )
        } else {
            None
        };

        if is_load {
            let addr = pop(stack, sp).ok_or(TcError::MmioTypedPopAddr { span: name_abs })?;
            match addr {
                Value::MmioPtr { reg, .. } => {
                    if !access_can_read(reg.access) {
                        return Err(TcError::MmioReadNotAllowed { span: name_abs });
                    }
                    if let Some(want) = ty_atom {
                        if want != reg.reg_ty {
                            return Err(TcError::MmioTypedTypeMismatch { span: name_abs });
                        }
                    }
                    check_atomic_width(reg.reg_ty, reg.meta, name_abs)?;
                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                    self.emit_op(
                        cur,
                        lir::OpKind::MmioVolLoad {
                            ty: tid,
                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                            read_kind: reg.meta.read_kind,
                            atomic_max: reg.meta.atomic_max,
                            barrier: reg.meta.barrier,
                        },
                        name_abs,
                    )?;
                    Ok(cur)
                }
                Value::MmioPlace(MmioResolved::Reg(reg)) => {
                    if !access_can_read(reg.access) {
                        return Err(TcError::MmioReadNotAllowed { span: name_abs });
                    }
                    if let Some(want) = ty_atom {
                        if want != reg.reg_ty {
                            return Err(TcError::MmioTypedTypeMismatch { span: name_abs });
                        }
                    }
                    check_atomic_width(reg.reg_ty, reg.meta, name_abs)?;
                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                    self.emit_op(
                        cur,
                        lir::OpKind::MmioVolLoad {
                            ty: tid,
                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                            read_kind: reg.meta.read_kind,
                            atomic_max: reg.meta.atomic_max,
                            barrier: reg.meta.barrier,
                        },
                        name_abs,
                    )?;
                    Ok(cur)
                }
                Value::MmioPlace(MmioResolved::Field(field)) => {
                    if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
                        return Err(TcError::MmioReadNotAllowed { span: name_abs });
                    }
                    if typed {
                        return Err(TcError::MmioTypedMismatch { span: name_abs });
                    }
                    check_atomic_width(field.reg_ty, field.meta, name_abs)?;
                    let (mask, shift) = field_mask_shift(&field.field);
                    push(stack, sp, Value::Plain(field.field.ty))?;
                    let reg_tid = self.ty_id_of_type(field.reg_ty, name_abs)?;
                    let tid = self.ty_id_of_type(field.field.ty, name_abs)?;
                    self.emit_op(
                        cur,
                        lir::OpKind::MmioVolLoadField {
                            reg_ty: reg_tid,
                            field_ty: tid,
                            place: lir_atom(slice_span(self.src, field.place_span))?,
                            mask,
                            shift,
                            read_kind: field.meta.read_kind,
                            atomic_max: field.meta.atomic_max,
                            barrier: field.meta.barrier,
                        },
                        name_abs,
                    )?;
                    Ok(cur)
                }
                Value::Ptr { ty, .. } => {
                    if !typed {
                        return Err(TcError::MmioTypedNotAllowed { span: name_abs });
                    }
                    let want = ty_atom.expect("typed => ty_atom is Some");
                    // Allow EMPTY pointee (raw pointer casts, S-8): the
                    // asserted type wins.
                    let resolved = if ty == TypeAtom::EMPTY { want } else { ty };
                    if want != resolved {
                        return Err(TcError::TypedLoadStoreTypeMismatch { span: name_abs });
                    }
                    push(stack, sp, Value::Plain(resolved))?;
                    let tid = self.ty_id_of_type(resolved, name_abs)?;
                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, name_abs)?;
                    Ok(cur)
                }
                _ => Err(TcError::MmioTypedNotAllowed { span: name_abs }),
            }
        } else {
            let v = pop(stack, sp).ok_or(TcError::ReturnStackDepth { span: name_abs })?;
            let addr = pop(stack, sp).ok_or(TcError::ReturnStackDepth { span: name_abs })?;
            let vty = v.to_type_atom();
            match (addr, v) {
                (
                    Value::MmioPtr {
                        reg,
                        mutable: false,
                    },
                    _,
                ) => {
                    let _ = reg;
                    Err(TcError::MmioTypedNotAllowed { span: name_abs })
                }
                (Value::MmioPtr { reg, mutable: true }, Value::Plain(_)) => {
                    if !access_can_write(reg.access) {
                        return Err(TcError::MmioAccessViolation { span: name_abs });
                    }
                    if let Some(want) = ty_atom {
                        if want != reg.reg_ty {
                            return Err(TcError::MmioTypedTypeMismatch { span: name_abs });
                        }
                    }
                    if vty != reg.reg_ty {
                        return Err(TcError::ReturnTypeMismatch { span: name_abs });
                    }
                    check_store_semantics(reg.reg_ty, reg.meta, name_abs)?;
                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                    self.emit_op(
                        cur,
                        lir::OpKind::MmioVolStore {
                            ty: tid,
                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                            write_kind: reg.meta.write_kind,
                            read_kind: reg.meta.read_kind,
                            atomic_max: reg.meta.atomic_max,
                            barrier: reg.meta.barrier,
                        },
                        name_abs,
                    )?;
                    Ok(cur)
                }
                (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(_)) => {
                    if !access_can_write(reg.access) {
                        return Err(TcError::MmioAccessViolation { span: name_abs });
                    }
                    if let Some(want) = ty_atom {
                        if want != reg.reg_ty {
                            return Err(TcError::MmioTypedTypeMismatch { span: name_abs });
                        }
                    }
                    if vty != reg.reg_ty {
                        return Err(TcError::ReturnTypeMismatch { span: name_abs });
                    }
                    check_store_semantics(reg.reg_ty, reg.meta, name_abs)?;
                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                    self.emit_op(
                        cur,
                        lir::OpKind::MmioVolStore {
                            ty: tid,
                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                            write_kind: reg.meta.write_kind,
                            read_kind: reg.meta.read_kind,
                            atomic_max: reg.meta.atomic_max,
                            barrier: reg.meta.barrier,
                        },
                        name_abs,
                    )?;
                    Ok(cur)
                }
                (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(_)) => {
                    if !access_can_write(field.reg_access) || !access_can_write(field.field.access)
                    {
                        return Err(TcError::MmioAccessViolation { span: name_abs });
                    }
                    if typed {
                        return Err(TcError::MmioTypedMismatch { span: name_abs });
                    }
                    if vty != field.field.ty {
                        return Err(TcError::ReturnTypeMismatch { span: name_abs });
                    }
                    // R1 (E3642): a field store is a read-modify-write; on an
                    // effectful register the RMW's read is a phantom bus read.
                    if field.meta.read_kind == ir::ReadKind::Effectful {
                        return Err(TcError::MmioPhantomRead { span: name_abs });
                    }
                    check_atomic_width(field.reg_ty, field.meta, name_abs)?;
                    let (mask, shift) = field_mask_shift(&field.field);
                    let reg_tid = self.ty_id_of_type(field.reg_ty, name_abs)?;
                    let tid = self.ty_id_of_type(field.field.ty, name_abs)?;
                    self.emit_op(
                        cur,
                        lir::OpKind::MmioVolStoreField {
                            reg_ty: reg_tid,
                            field_ty: tid,
                            place: lir_atom(slice_span(self.src, field.place_span))?,
                            mask,
                            shift,
                            write_kind: field.meta.write_kind,
                            read_kind: field.meta.read_kind,
                            atomic_max: field.meta.atomic_max,
                            barrier: field.meta.barrier,
                        },
                        name_abs,
                    )?;
                    Ok(cur)
                }
                (Value::Ptr { ty, mutable, .. }, Value::Plain(_)) => {
                    if !typed {
                        return Err(TcError::MmioTypedNotAllowed { span: name_abs });
                    }
                    if !mutable {
                        return Err(TcError::MmioTypedNotAllowed { span: name_abs });
                    }
                    let want = ty_atom.expect("typed => ty_atom is Some");
                    let resolved = if ty == TypeAtom::EMPTY { want } else { ty };
                    if want != resolved {
                        return Err(TcError::TypedLoadStoreTypeMismatch { span: name_abs });
                    }
                    if vty != resolved {
                        return Err(TcError::ReturnTypeMismatch { span: name_abs });
                    }
                    let tid = self.ty_id_of_type(resolved, name_abs)?;
                    self.emit_op(cur, lir::OpKind::Store { ty: tid }, name_abs)?;
                    Ok(cur)
                }
                _ => Err(TcError::MmioTypedNotAllowed { span: name_abs }),
            }
        }
    }
}
