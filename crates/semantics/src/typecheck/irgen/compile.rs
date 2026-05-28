use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn compile_quote_span(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        quot_span: Span,
        allow_suspend: bool,
        allow_locals: bool,
    ) -> Result<lir::BlockId, TcError> {
        if quot_span.end <= quot_span.start + 2 {
            return Ok(cur);
        }
        let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
        self.compile_span(cur, stack, sp, inner, allow_suspend, allow_locals)
    }

    pub(super) fn compile_span(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        allow_suspend: bool,
        allow_locals: bool,
    ) -> Result<lir::BlockId, TcError> {
        let slice = &self.src[span.start..span.end];
        let mut lex = Lexer::new(slice);
        let mut terminated = false;

        loop {
            let tok = lex.next();
            if tok.kind == TokenKind::Eof {
                break;
            }
            if terminated {
                continue;
            }

            match tok.kind {
                TokenKind::Number => {
                    push(stack, sp, Value::Plain(TypeAtom::I64))?;
                    let num = parse_i64_token(&slice[tok.span.start..tok.span.end]).unwrap_or(0);
                    self.emit_op(cur, lir::OpKind::ConstI64(num), Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                }
                TokenKind::String => {
                    push(stack, sp, Value::Plain(TypeAtom::STR))?;
                    self.emit_op(cur, lir::OpKind::ConstStr(Span::new(span.start + tok.span.start, span.start + tok.span.end)), Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                }
                TokenKind::PunctArrowBind => {
                    if !allow_locals {
                        return Err(TcError { code: 3281, span });
                    }
                    let next = lex.next();
                    if next.kind == TokenKind::PunctLBrace {
                        let mut binds: FixedVec<DestructBind, 32> = FixedVec::new();
                        let mut saw_borrow = false;
                        loop {
                            let b = lex.next();
                            match b.kind {
                                TokenKind::PunctRBrace => break,
                                TokenKind::PunctComma => continue,
                                TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                                    saw_borrow = true;
                                    let name_tok = lex.next();
                                    if name_tok.kind != TokenKind::Ident {
                                        return Err(TcError { code: 3201, span: Span::new(span.start + name_tok.span.start, span.start + name_tok.span.end) });
                                    }
                                    let lname = TypeAtom::new(&slice[name_tok.span.start..name_tok.span.end]).ok_or(TcError {
                                        code: 3203,
                                        span: Span::new(span.start + name_tok.span.start, span.start + name_tok.span.end),
                                    })?;
                                    binds
                                        .push(DestructBind {
                                            name: lname,
                                            borrow: true,
                                            mutable: b.kind == TokenKind::PunctAmpBang,
                                            span: Span::new(span.start + name_tok.span.start, span.start + name_tok.span.end),
                                        })
                                        .map_err(|_| TcError { code: 3205, span })?;
                                }
                                TokenKind::Ident => {
                                    if saw_borrow {
                                        return Err(TcError { code: 3701, span: Span::new(span.start + b.span.start, span.start + b.span.end) });
                                    }
                                    let lname = TypeAtom::new(&slice[b.span.start..b.span.end]).ok_or(TcError {
                                        code: 3203,
                                        span: Span::new(span.start + b.span.start, span.start + b.span.end),
                                    })?;
                                    binds
                                        .push(DestructBind {
                                            name: lname,
                                            borrow: false,
                                            mutable: false,
                                            span: Span::new(span.start + b.span.start, span.start + b.span.end),
                                        })
                                        .map_err(|_| TcError { code: 3205, span })?;
                                }
                                _ => {
                                    return Err(TcError { code: 3702, span: Span::new(span.start + b.span.start, span.start + b.span.end) });
                                }
                            }
                        }
                        if binds.is_empty() {
                            return Err(TcError { code: 3703, span });
                        }
                        let base = *stack.get(*sp - 1).ok_or(TcError { code: 3202, span })?;
                        let has_borrow = binds.iter().any(|b| b.borrow);
                        let (struct_ty, base_mut, _base_is_value) = match base {
                            Value::Ptr { ty, mutable } => (ty, mutable, false),
                            Value::Plain(t) if !has_borrow => (t, false, true),
                            _ => return Err(TcError { code: 3704, span }),
                        };
                        let Some(sinfo) = self.nominals.structs.iter().find(|s| s.name == struct_ty) else {
                            return Err(TcError { code: 3716, span });
                        };
                        if binds.len() != sinfo.fields.len() {
                            return Err(TcError { code: 3705, span });
                        }
                        let base_tid = if base_mut { lir::TY_PTR_MUT } else { lir::TY_PTR };
                        let tmp = self.temp_base_slot();
                        self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: base_tid }, Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                        let _ = pop(stack, sp);

                        let mut offset: u32 = 0;
                        for (idx, field) in sinfo.fields.iter().enumerate() {
                            let bind = binds.get(idx).expect("len verified equal to sinfo.fields.len()");
                            if bind.borrow && bind.mutable && !base_mut {
                                return Err(TcError { code: 3501, span: bind.span });
                            }
                            let fsize = type_size_bytes(field.ty, self.nominals).ok_or(TcError { code: 3718, span: bind.span })?;
                            let falign = field_align(fsize);
                            offset = align_up(offset, falign);
                            let field_offset = offset;
                            offset = offset.checked_add(fsize).ok_or(TcError { code: 3718, span: bind.span })?;

                            self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: base_tid }, bind.span)?;
                            push(stack, sp, Value::Ptr { ty: struct_ty, mutable: base_mut })?;
                            self.emit_op(cur, lir::OpKind::PtrAddConst { ty: base_tid, offset: field_offset }, bind.span)?;

                            if bind.borrow {
                                let ptr_ty = if bind.mutable { TypeAtom::PTR_MUT } else { TypeAtom::PTR };
                                if find_local(&self.locals, self.local_len, bind.name).is_some() {
                                    return Err(TcError { code: 3204, span: bind.span });
                                }
                                let slot = self.local_slot(self.local_len);
                                self.locals[self.local_len] = bind.name;
                                self.local_tys[self.local_len] = ptr_ty;
                                self.local_live[self.local_len] = true;
                                self.local_scoped[self.local_len] = 0u16;
                                self.local_len += 1;
                                let tid = if bind.mutable { lir::TY_PTR_MUT } else { lir::TY_PTR };
                                let _ = pop(stack, sp);
                                self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, bind.span)?;
                            } else {
                                let tid = self.ty_id_of_type(field.ty, bind.span)?;
                                self.emit_op(cur, lir::OpKind::Load { ty: tid }, bind.span)?;
                                let _ = pop(stack, sp);
                                push(stack, sp, Value::Plain(field.ty))?;
                                if find_local(&self.locals, self.local_len, bind.name).is_some() {
                                    return Err(TcError { code: 3204, span: bind.span });
                                }
                                let slot = self.local_slot(self.local_len);
                                self.locals[self.local_len] = bind.name;
                                self.local_tys[self.local_len] = field.ty;
                                self.local_live[self.local_len] = true;
                                self.local_scoped[self.local_len] = 0u16;
                                self.local_len += 1;
                                let _ = pop(stack, sp);
                                self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, bind.span)?;
                            }
                        }
                    } else {
                        let name = next;
                        if name.kind != TokenKind::Ident {
                            return Err(TcError { code: 3201, span: Span::new(span.start + name.span.start, span.start + name.span.end) });
                        }
                        let v = pop(stack, sp).ok_or(TcError { code: 3202, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
                        if v == Value::Plain(TypeAtom::SCOPED) {
                            return Err(TcError { code: 3504, span });
                        }
                        let ty = match v {
                            Value::Plain(t) => t,
                            Value::Scoped { ty, .. } => ty,
                            Value::Resource(_) => TypeAtom::RESOURCE,
                            Value::Quot(_) => TypeAtom::QUOT,
                            Value::MmioPlace(_) => TypeAtom::MMIO,
                            Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                            Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                            Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                            Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                        };
                        let lname = TypeAtom::new(&slice[name.span.start..name.span.end]).ok_or(TcError {
                            code: 3203,
                            span: Span::new(span.start + name.span.start, span.start + name.span.end),
                        })?;
                        if find_local(&self.locals, self.local_len, lname).is_some() {
                            return Err(TcError { code: 3204, span: Span::new(span.start + name.span.start, span.start + name.span.end) });
                        }
                        if self.local_len >= self.locals.len() {
                            return Err(TcError { code: 3205, span });
                        }
                        let slot = self.local_slot(self.local_len);
                        self.locals[self.local_len] = lname;
                        self.local_tys[self.local_len] = ty;
                        self.local_live[self.local_len] = true;
                        self.local_scoped[self.local_len] = match v {
                            Value::Scoped { scope, .. } => scope,
                            _ => 0u16,
                        };
                        self.local_len += 1;
                        let tid = self.ty_id_of_type(ty, span)?;
                        self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                    }
                }
                TokenKind::PunctLBracket => {
                    let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                        .map_err(|code| TcError { code, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
                    let q_span = Span::new(span.start + q.start, span.start + q.end);
                    push(stack, sp, Value::Quot(q_span))?;
                }
                TokenKind::PunctArrow => {
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let field_tok = lex.next();
                    if field_tok.kind != TokenKind::Ident {
                        return Err(TcError { code: 3716, span: op_span });
                    }
                    let field_atom = TypeAtom::new(&slice[field_tok.span.start..field_tok.span.end])
                        .ok_or(TcError { code: 3716, span: op_span })?;
                    let base = *stack.get(*sp - 1).ok_or(TcError { code: 3202, span: op_span })?;
                    let (struct_ty, mutable) = match base {
                        Value::Ptr { ty, mutable } => (ty, mutable),
                        _ => return Err(TcError { code: 3716, span: op_span }),
                    };
                    let Some(sinfo) = self.nominals.structs.iter().find(|s| s.name == struct_ty) else {
                        return Err(TcError { code: 3716, span: op_span });
                    };
                    let mut offset: u32 = 0;
                    let mut found: Option<TypeAtom> = None;
                    for field in sinfo.fields.iter() {
                        let fsize = type_size_bytes(field.ty, self.nominals).ok_or(TcError { code: 3718, span: op_span })?;
                        let falign = field_align(fsize);
                        offset = align_up(offset, falign);
                        if field.name == field_atom {
                            found = Some(field.ty);
                            break;
                        }
                        offset = offset.checked_add(fsize).ok_or(TcError { code: 3718, span: op_span })?;
                    }
                    let Some(field_ty) = found else {
                        return Err(TcError { code: 3716, span: op_span });
                    };
                    let base_tid = if mutable { lir::TY_PTR_MUT } else { lir::TY_PTR };
                    self.emit_op(cur, lir::OpKind::PtrAddConst { ty: base_tid, offset }, op_span)?;
                    stack[*sp - 1] = Value::Ptr { ty: field_ty, mutable };
                }
                TokenKind::PunctApostrophe => {
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let mut const_idx: Option<u32> = None;
                    let mut is_dynamic = false;

                    let next = lex.next();
                    match next.kind {
                        TokenKind::Number => {
                            const_idx = parse_u32_any(&slice[next.span.start..next.span.end]);
                            if const_idx.is_none() {
                                return Err(TcError { code: 3519, span: op_span });
                            }
                        }
                        TokenKind::PunctLParen => {
                            let par = capture_balanced(&mut lex, slice, TokenKind::PunctLParen, TokenKind::PunctRParen, next.span.start)
                                .map_err(|code| TcError { code, span: op_span })?;
                            let inner = Span::new(span.start + par.start + 1, span.start + par.end - 1);
                            cur = self.compile_span(cur, stack, sp, inner, allow_suspend, allow_locals)?;
                            is_dynamic = true;
                        }
                        _ => {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                    }

                    if is_dynamic {
                        if *sp == 0 {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                        if stack[*sp - 1] != Value::Plain(TypeAtom::I64) {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                    }

                    let base_pos = if is_dynamic { *sp - 2 } else { *sp - 1 };
                    if base_pos >= *sp {
                        return Err(TcError { code: 3519, span: op_span });
                    }

                    let base = stack[base_pos];
                    let (elem_ty, scale, out_kind, base_tid) = match base {
                        Value::Plain(t) => {
                            let Some(elem) = array_elem_type(t) else {
                                return Err(TcError { code: 3519, span: op_span });
                            };
                            let len = array_len(t).ok_or(TcError { code: 3519, span: op_span })?;
                            if let Some(idx) = const_idx {
                                if idx >= len {
                                    return Err(TcError { code: 3518, span: op_span });
                                }
                            }
                            let size = type_size_bytes(elem, self.nominals).ok_or(TcError { code: 3519, span: op_span })?;
                            (elem, size, IndexOut::Value, lir::TY_PTR)
                        }
                        Value::Ptr { ty, mutable } => {
                            let Some(elem) = array_elem_type(ty) else {
                                return Err(TcError { code: 3519, span: op_span });
                            };
                            let len = array_len(ty).ok_or(TcError { code: 3519, span: op_span })?;
                            if let Some(idx) = const_idx {
                                if idx >= len {
                                    return Err(TcError { code: 3518, span: op_span });
                                }
                            }
                            let size = type_size_bytes(elem, self.nominals).ok_or(TcError { code: 3519, span: op_span })?;
                            let tid = if mutable { lir::TY_PTR_MUT } else { lir::TY_PTR };
                            (elem, size, IndexOut::Ptr(mutable), tid)
                        }
                        Value::MmioPlace(MmioResolved::Reg(reg)) => {
                            let Some(width) = mmio_type_width_bytes(reg.reg_ty.as_bytes()) else {
                                return Err(TcError { code: 3519, span: op_span });
                            };
                            if let Some(idx) = const_idx {
                                if let Some(len) = reg.array_len {
                                    if idx >= len {
                                        return Err(TcError { code: 3604, span: op_span });
                                    }
                                }
                            }
                            (reg.reg_ty, width, IndexOut::Mmio, lir::TY_MMIO)
                        }
                        Value::MmioPlace(MmioResolved::Field(_)) => {
                            return Err(TcError { code: 3519, span: op_span });
                        }
                        _ => return Err(TcError { code: 3519, span: op_span }),
                    };

                    match out_kind {
                        IndexOut::Value => {
                            stack[base_pos] = Value::Ptr { ty: elem_ty, mutable: false };
                        }
                        IndexOut::Ptr(mutable) => {
                            stack[base_pos] = Value::Ptr { ty: elem_ty, mutable };
                        }
                        IndexOut::Mmio => {
                        }
                    }

                    if is_dynamic {
                        self.emit_op(cur, lir::OpKind::PtrAddIndex { ty: base_tid, scale }, op_span)?;
                        *sp = (*sp).saturating_sub(1);
                    } else {
                        let offset = const_idx.expect("!is_dynamic => const_idx is Some").saturating_mul(scale);
                        self.emit_op(cur, lir::OpKind::PtrAddConst { ty: base_tid, offset }, op_span)?;
                    }

                    if matches!(out_kind, IndexOut::Value) {
                        let tid = self.ty_id_of_type(elem_ty, op_span)?;
                        self.emit_op(cur, lir::OpKind::Load { ty: tid }, op_span)?;
                        stack[base_pos] = Value::Plain(elem_ty);
                    }
                }
                TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                    let mut_tok = tok.kind == TokenKind::PunctAmpBang;
                    let place = parse_place(&mut lex, slice).ok_or(TcError { code: 3500, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
                    let place_bytes = &slice[place.full.start..place.full.end];
                    let place_abs = Span::new(span.start + place.full.start, span.start + place.full.end);
                    let root_atom = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap_or(TypeAtom::EMPTY);
                    let mut const_addr: Option<u64> = None;

                    if let Some(res) = resolve_mmio_place(self.mmio, self.src, place_bytes, place_abs)? {
                        match res {
                            MmioResolved::Reg(reg) => {
                                if mut_tok && !access_can_write(reg.access) {
                                    return Err(TcError { code: 3609, span: place_abs });
                                }
                                const_addr = Some(reg.addr);
                                push(stack, sp, Value::MmioPtr { reg, mutable: mut_tok })?;
                            }
                            MmioResolved::Field(_) => return Err(TcError { code: 3608, span: place_abs }),
                        }
                    } else {
                        if resource_ty(self.resources, root_atom).is_some() {
                            if self.locked_resource != Some(root_atom) {
                                return Err(TcError { code: 3515, span: place_abs });
                            }
                        } else if mut_tok {
                            let root = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap();
                            if find_local(&self.locals, self.local_len, root).is_some() {
                                return Err(TcError { code: 3501, span: place.root_abs(span.start) });
                            }
                        }
                        if let Some(pointee) = self.resolve_place_pointee_ty(place_bytes, place_abs)? {
                            push(stack, sp, Value::Ptr { ty: pointee, mutable: mut_tok })?;
                        } else {
                            let ty = if mut_tok { TypeAtom::PTR_MUT } else { TypeAtom::PTR };
                            push(stack, sp, Value::Plain(ty))?;
                        }
                    }

                    self.emit_op(
                        cur,
                        lir::OpKind::AddrOf {
                            place: lir_atom(place_bytes)?,
                            mutable: mut_tok,
                            const_addr,
                        },
                        place_abs,
                    )?;
                }
                TokenKind::PunctPipeGreater => {
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let val = pop(stack, sp).ok_or(TcError { code: 3730, span: op_span })?;
                    let ch = pop(stack, sp).ok_or(TcError { code: 3730, span: op_span })?;

                    let val_ty = match val {
                        Value::Plain(t) => t,
                        Value::Scoped { ty, .. } => ty,
                        Value::Resource(_) => TypeAtom::RESOURCE,
                        Value::Quot(_) => TypeAtom::QUOT,
                        Value::MmioPlace(_) => TypeAtom::MMIO,
                        Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                        Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                        Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                        Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                    };
                    let ch_ty = match ch {
                        Value::Plain(t) => t,
                        _ => return Err(TcError { code: 3731, span: op_span }),
                    };
                    let elem = chan_elem_type(ch_ty).ok_or(TcError { code: 3731, span: op_span })?;
                    if !type_compatible(val_ty, elem, self.subtypes) {
                        return Err(TcError { code: 3732, span: op_span });
                    }

                    let ch_tid = self.ty_id_of_type(ch_ty, op_span)?;
                    let val_tid = self.ty_id_of_type(val_ty, op_span)?;
                    let mut sig = lir::Sig::empty();
                    sig.in_len = 2;
                    sig.out_len = 0;
                    sig.inputs[0] = ch_tid;
                    sig.inputs[1] = val_tid;
                    self.emit_op(
                        cur,
                        lir::OpKind::Call {
                            name: lir::Atom::new(b"platform.channel.send").unwrap(),
                            sig,
                            may_suspend: false,
                        },
                        op_span,
                    )?;
                }
                TokenKind::PunctLessPipe => {
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let ch = pop(stack, sp).ok_or(TcError { code: 3733, span: op_span })?;
                    let ch_ty = match ch {
                        Value::Plain(t) => t,
                        _ => return Err(TcError { code: 3734, span: op_span }),
                    };
                    let elem = chan_elem_type(ch_ty).ok_or(TcError { code: 3734, span: op_span })?;
                    push(stack, sp, Value::Plain(elem))?;

                    let ch_tid = self.ty_id_of_type(ch_ty, op_span)?;
                    let elem_tid = self.ty_id_of_type(elem, op_span)?;
                    let mut sig = lir::Sig::empty();
                    sig.in_len = 1;
                    sig.out_len = 1;
                    sig.inputs[0] = ch_tid;
                    sig.outputs[0] = elem_tid;
                    self.emit_op(
                        cur,
                        lir::OpKind::Call {
                            name: lir::Atom::new(b"platform.channel.recv").unwrap(),
                            sig,
                            may_suspend: false,
                        },
                        op_span,
                    )?;
                }
                TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                    let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
                    if *sp == 0 {
                        return Err(TcError { code: 3505, span });
                    }
                    let top = stack[*sp - 1];
                    let top_ty = match top {
                        Value::Plain(t) => t,
                        Value::Scoped { ty, .. } => ty,
                        Value::Resource(_) => TypeAtom::RESOURCE,
                        Value::Quot(_) => TypeAtom::QUOT,
                        Value::MmioPlace(_) => TypeAtom::MMIO,
                        Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                        Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                        Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                        Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                    };

                    if let Some(elem) = array_elem_type(top_ty) {
                        let scope_id = self.enter_scope().ok_or(TcError { code: 3512, span })?;
                        let slice_ty = slice_type_of_elem(elem, mut_scope).ok_or(TcError { code: 3513, span })?;
                        let len = array_len(top_ty).ok_or(TcError { code: 3513, span })?;
                        push(
                            stack,
                            sp,
                            Value::Scoped {
                                ty: slice_ty,
                                scope: scope_id,
                            },
                        )?;
                        let tid = self.ty_id_of_type(slice_ty, span)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::ScopedEnter { ty: tid, len },
                            Span::new(span.start + tok.span.start, span.start + tok.span.end),
                        )?;

                        let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span })?;
                        let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                        cur = self.compile_span(
                            cur,
                            stack,
                            sp,
                            Span::new(span.start + block.inner_start, span.start + block.inner_end),
                            block_allow_suspend,
                            allow_locals,
                        )?;

                        if self.stack_has_scope(stack, *sp, scope_id) {
                            return Err(TcError { code: 3506, span });
                        }
                        self.invalidate_scope_locals(scope_id);
                        self.leave_scope(scope_id);
                    } else if top_ty == TypeAtom::new(b"Region").unwrap() {
                        let scope_id = self.enter_scope().ok_or(TcError { code: 3512, span })?;
                        let rty = region_ref_type(mut_scope);
                        push(
                            stack,
                            sp,
                            Value::Scoped {
                                ty: rty,
                                scope: scope_id,
                            },
                        )?;
                        let tid = self.ty_id_of_type(rty, span)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::ScopedEnter { ty: tid, len: 0 },
                            Span::new(span.start + tok.span.start, span.start + tok.span.end),
                        )?;

                        let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span })?;
                        let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                        cur = self.compile_span(
                            cur,
                            stack,
                            sp,
                            Span::new(span.start + block.inner_start, span.start + block.inner_end),
                            block_allow_suspend,
                            allow_locals,
                        )?;

                        if self.stack_has_scope(stack, *sp, scope_id) {
                            return Err(TcError { code: 3506, span });
                        }
                        self.invalidate_scope_locals(scope_id);
                        self.leave_scope(scope_id);
                    } else {
                        return Err(TcError { code: 3515, span });
                    }
                }
                TokenKind::Ident | TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                    let mut qbuf = [0u8; 64];
                    let (name, name_span) = if tok.kind == TokenKind::Ident {
                        let (len, used, s) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                        let bytes = if used { &qbuf[..len] } else { &slice[tok.span.start..tok.span.end] };
                        (bytes, s)
                    } else {
                        (&slice[tok.span.start..tok.span.end], tok.span)
                    };
                    let name_abs = Span::new(span.start + name_span.start, span.start + name_span.end);

                    if name == b"true" || name == b"false" {
                        push(stack, sp, Value::Plain(TypeAtom::BOOL))?;
                        self.emit_op(cur, lir::OpKind::ConstBool(name == b"true"), name_abs)?;
                        continue;
                    }

                    if tok.kind == TokenKind::Ident {
                        if let Some(atom) = TypeAtom::new(name) {
                            if resource_ty(self.resources, atom).is_some() {
                                push(stack, sp, Value::Resource(atom))?;
                                continue;
                            }
                        }
                    }

                    if tok.kind == TokenKind::Ident {
                        if let Some(dot) = name.iter().position(|&b| b == b'.') {
                            if dot + 1 < name.len() && !name[dot + 1..].contains(&b'.') {
                                let enum_part = &name[..dot];
                                let var_part = &name[dot + 1..];
                                if let (Some(enum_ty), Some(var)) = (TypeAtom::new(enum_part), TypeAtom::new(var_part)) {
                                    if let Some(v) = enum_variant_value(self.nominals, enum_ty, var) {
                                        self.emit_op(cur, lir::OpKind::ConstI64(v), name_abs)?;
                                        push(stack, sp, Value::Plain(TypeAtom::I64))?;
                                        let to = self.ty_id_of_type(enum_ty, name_abs)?;
                                        self.emit_op(cur, lir::OpKind::Cast { from: lir::TY_I64, to }, name_abs)?;
                                        let _ = pop(stack, sp);
                                        push(stack, sp, Value::Plain(enum_ty))?;
                                        continue;
                                    }
                                    for e in self.nominals.enums.iter() {
                                        if e.name == enum_ty {
                                            return Err(TcError { code: 3725, span: name_abs });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
                        let is_load = name[0] == b'@';
                        let typed = name.len() > 1;
                        let ty_atom = if typed {
                            Some(TypeAtom::new(&name[1..]).ok_or(TcError { code: 3632, span: name_abs })?)
                        } else {
                            None
                        };

                        if is_load {
                            let addr = pop(stack, sp).ok_or(TcError { code: 3633, span: name_abs })?;
                            match addr {
                                Value::MmioPtr { reg, .. } => {
                                    if !access_can_read(reg.access) {
                                        return Err(TcError { code: 3610, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolLoad {
                                            ty: tid,
                                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                    if !access_can_read(reg.access) {
                                        return Err(TcError { code: 3610, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolLoad {
                                            ty: tid,
                                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                Value::MmioPlace(MmioResolved::Field(field)) => {
                                    if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
                                        return Err(TcError { code: 3610, span: name_abs });
                                    }
                                    if typed {
                                        return Err(TcError { code: 3634, span: name_abs });
                                    }
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
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                Value::Ptr { ty, .. } => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let want = ty_atom.expect("typed => ty_atom is Some");
                                    if want != ty {
                                        return Err(TcError { code: 3717, span: name_abs });
                                    }
                                    push(stack, sp, Value::Plain(ty))?;
                                    let tid = self.ty_id_of_type(ty, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, name_abs)?;
                                    continue;
                                }
                                Value::Plain(t) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if t != TypeAtom::PTR && t != TypeAtom::PTR_MUT {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let ty_atom = ty_atom.expect("typed => ty_atom is Some");
                                    push(stack, sp, Value::Plain(ty_atom))?;
                                    let tid = self.ty_id_of_type(ty_atom, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, name_abs)?;
                                    continue;
                                }
                                _ => return Err(TcError { code: 3614, span: name_abs }),
                            }
                        } else {
                            let v = pop(stack, sp).ok_or(TcError { code: 3230, span: name_abs })?;
                            let addr = pop(stack, sp).ok_or(TcError { code: 3230, span: name_abs })?;
                            let vty = match v {
                                Value::Plain(t) => t,
                                Value::Scoped { ty, .. } => ty,
                                Value::Resource(_) => TypeAtom::RESOURCE,
                                Value::Quot(_) => TypeAtom::QUOT,
                                Value::MmioPlace(_) => TypeAtom::MMIO,
                                Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                                Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                                Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                                Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                            };
                            match (addr, v) {
                                (Value::MmioPtr { reg, mutable: false }, _) => {
                                    let _ = reg;
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                (Value::MmioPtr { reg, mutable: true }, Value::Plain(_)) => {
                                    if !access_can_write(reg.access) {
                                        return Err(TcError { code: 3609, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    if vty != reg.reg_ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolStore {
                                            ty: tid,
                                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(_)) => {
                                    if !access_can_write(reg.access) {
                                        return Err(TcError { code: 3609, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    if vty != reg.reg_ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolStore {
                                            ty: tid,
                                            place: lir_atom(slice_span(self.src, reg.place_span))?,
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(_)) => {
                                    if !access_can_write(field.reg_access) || !access_can_write(field.field.access) {
                                        return Err(TcError { code: 3609, span: name_abs });
                                    }
                                    if typed {
                                        return Err(TcError { code: 3634, span: name_abs });
                                    }
                                    if vty != field.field.ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
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
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                (Value::Ptr { ty, mutable }, Value::Plain(_)) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if !mutable {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let want = ty_atom.expect("typed => ty_atom is Some");
                                    if want != ty {
                                        return Err(TcError { code: 3717, span: name_abs });
                                    }
                                    if vty != ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(ty, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Store { ty: tid }, name_abs)?;
                                    continue;
                                }
                                (Value::Plain(t), Value::Plain(_)) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if t != TypeAtom::PTR_MUT {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let ty_atom = ty_atom.expect("typed => ty_atom is Some");
                                    if vty != ty_atom {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(ty_atom, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Store { ty: tid }, name_abs)?;
                                    continue;
                                }
                                _ => return Err(TcError { code: 3614, span: name_abs }),
                            }
                        }
                    }

                    if tok.kind == TokenKind::Ident {
                        if let Some(res) = resolve_mmio_place(self.mmio, self.src, name, name_abs)? {
                            let addr = match res {
                                MmioResolved::Reg(reg) => reg.addr,
                                MmioResolved::Field(field) => field.addr,
                            };
                            push(stack, sp, Value::MmioPlace(res))?;
                            self.emit_op(
                                cur,
                                lir::OpKind::MmioPlace {
                                    place: lir_atom(name)?,
                                    addr,
                                },
                                name_abs,
                            )?;
                            continue;
                        }
                    }

                    if name == b"dup" {
                        let top = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        let top_ty = match top {
                            Value::Plain(t) => t,
                            Value::Scoped { ty, .. } => ty,
                            Value::Resource(_) => TypeAtom::RESOURCE,
                            Value::Quot(_) => TypeAtom::QUOT,
                            Value::MmioPlace(_) => TypeAtom::MMIO,
                            Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                            Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                            Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                            Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                        };
                        if is_iso_type(self.iso, top_ty) {
                            return Err(TcError { code: 3742, span: name_abs });
                        }
                        push(stack, sp, top)?;
                        push(stack, sp, top)?;
                        let tid = self.ty_id_of_value(top, name_abs)?;
                        self.emit_op(cur, lir::OpKind::Dup { ty: tid }, name_abs)?;
                        continue;
                    }
                    if name == b"drop" {
                        let top = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        let top_ty = match top {
                            Value::Plain(t) => t,
                            Value::Scoped { ty, .. } => ty,
                            Value::Resource(_) => TypeAtom::RESOURCE,
                            Value::Quot(_) => TypeAtom::QUOT,
                            Value::MmioPlace(_) => TypeAtom::MMIO,
                            Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                            Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                            Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                            Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                        };
                        if is_iso_type(self.iso, top_ty) {
                            return Err(TcError { code: 3743, span: name_abs });
                        }
                        let tid = self.ty_id_of_value(top, name_abs)?;
                        self.emit_op(cur, lir::OpKind::Drop { ty: tid }, name_abs)?;
                        continue;
                    }
                    if name == b"swap" {
                        let b = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        let a = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        push(stack, sp, b)?;
                        push(stack, sp, a)?;
                        let a_id = self.ty_id_of_value(a, name_abs)?;
                        let b_id = self.ty_id_of_value(b, name_abs)?;
                        self.emit_op(cur, lir::OpKind::Swap { a: a_id, b: b_id }, name_abs)?;
                        continue;
                    }

                    if name == b"as" || name == b"as?" || name == b"bitcast" {
                        let first = lex.next();
                        let start = first.span.start;
                        let (to_ty, next) = crate::typecheck::parse::parse_type_expr(slice, start).ok_or(TcError {
                            code: 3295,
                            span: Span::new(span.start + first.span.start, span.start + first.span.end),
                        })?;
                        lex.set_pos(next);
                        let v = pop(stack, sp).ok_or(TcError { code: 3297, span })?;
                        let from_ty = match v {
                            Value::Plain(t) => t,
                            Value::Scoped { ty, .. } => ty,
                            Value::Resource(_) => TypeAtom::RESOURCE,
                            Value::Quot(_) => TypeAtom::QUOT,
                            Value::MmioPlace(_) => TypeAtom::MMIO,
                            Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                            Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                            Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                            Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                        };
                        let subtype = find_subtype(self.subtypes, to_ty);
                        if let Some(st) = subtype {
                            if !type_compatible(from_ty, st.base, self.subtypes) {
                                return Err(TcError { code: if name == b"as?" { 3298 } else { 3300 }, span });
                            }
                        }

                        if name == b"bitcast" {
                            if subtype.is_some() {
                                return Err(TcError { code: 3302, span: name_abs });
                            }
                            let Some((from_bits, _)) = self.ty_bits_signed(from_ty) else {
                                return Err(TcError { code: 3303, span: name_abs });
                            };
                            let Some((to_bits, _)) = self.ty_bits_signed(to_ty) else {
                                return Err(TcError { code: 3303, span: name_abs });
                            };
                            if from_bits != to_bits {
                                return Err(TcError { code: 3304, span: name_abs });
                            }
                            if !self.check_raw_cast_allowed(from_ty, to_ty) {
                                return Err(TcError { code: 3305, span: name_abs });
                            }
                        } else {
                            if !self.check_raw_cast_allowed(from_ty, to_ty) {
                                return Err(TcError { code: 3305, span: name_abs });
                            }
                        }

                        let from_id = self.ty_id_of_type(from_ty, name_abs)?;
                        let to_id = self.ty_id_of_type(to_ty, name_abs)?;
                        if name == b"bitcast" {
                            self.emit_op(cur, lir::OpKind::Bitcast { from: from_id, to: to_id }, name_abs)?;
                        } else {
                            self.emit_op(cur, lir::OpKind::Cast { from: from_id, to: to_id }, name_abs)?;
                        }
                        if name == b"as?" {
                            push(stack, sp, Value::Plain(to_ty))?;
                            if let Some(st) = subtype {
                                let tmp = self.temp_base_slot();
                                self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: to_id }, name_abs)?;

                                self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::ConstI64(st.min), name_abs)?;
                                self.emit_op(
                                    cur,
                                    lir::OpKind::Cmp {
                                        out: lir::TY_BOOL,
                                        kind: lir::CmpKind::Ge,
                                    },
                                    name_abs,
                                )?;
                                self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::ConstI64(st.max), name_abs)?;
                                self.emit_op(
                                    cur,
                                    lir::OpKind::Cmp {
                                        out: lir::TY_BOOL,
                                        kind: lir::CmpKind::Le,
                                    },
                                    name_abs,
                                )?;
                                self.emit_op(cur, lir::OpKind::AndBool, name_abs)?;
                                push(stack, sp, Value::Plain(TypeAtom::BOOL))?;
                            } else {
                                self.emit_op(cur, lir::OpKind::ConstBool(true), name_abs)?;
                                push(stack, sp, Value::Plain(TypeAtom::BOOL))?;
                            }
                            continue;
                        }
                        push(stack, sp, Value::Plain(to_ty))?;
                        if name == b"as" {
                            if let Some(st) = subtype {
                                if self.checks == ChecksMode::All {
                                    let tmp = self.temp_base_slot();
                                    self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: to_id }, name_abs)?;
                                    self.emit_subtype_range_trap(cur, tmp, to_id, &st, name_abs)?;
                                }
                            }
                        }
                        continue;
                    }

                    if name == b"if" {
                        cur = self.compile_if(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"while" {
                        cur = self.compile_while(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"loop" {
                        cur = self.compile_loop(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"return" {
                        let want = self.sig.out_len as usize;
                        if *sp != want {
                            return Err(TcError { code: 3230, span });
                        }
                        if self.any_scoped_live(stack, *sp) {
                            return Err(TcError { code: 3511, span });
                        }
                        for (i, v) in stack.iter().enumerate().take(want) {
                            let got = match v {
                                Value::Plain(t) => *t,
                                Value::Scoped { ty, .. } => *ty,
                                Value::Resource(_) => TypeAtom::RESOURCE,
                                Value::Quot(_) => TypeAtom::QUOT,
                                Value::MmioPlace(_) => TypeAtom::MMIO,
                                Value::Ptr { mutable: false, .. } => TypeAtom::PTR,
                                Value::Ptr { mutable: true, .. } => TypeAtom::PTR_MUT,
                                Value::MmioPtr { mutable: false, .. } => TypeAtom::PTR,
                                Value::MmioPtr { mutable: true, .. } => TypeAtom::PTR_MUT,
                            };
                            if !type_compatible(got, self.sig.outputs[i], self.subtypes) {
                                return Err(TcError { code: 3231, span });
                            }
                        }
                        self.emit_op(cur, lir::OpKind::Ret, name_abs)?;
                        terminated = true;
                        self.terminated = true;
                        continue;
                    }
                    if name == b"lock" {
                        let mut probe = lex;
                        let next = probe.next();
                        if next.kind == TokenKind::PunctLBracket {
                            lex = probe;
                            let _block = capture_scoped_block(&mut lex, slice, next.span)
                                .map_err(|code| TcError { code, span: name_abs })?;
                            let full_span = Span::new(span.start + next.span.start, span.start + lex.pos());
                            push(stack, sp, Value::Quot(full_span))?;
                        }
                        cur = self.compile_lock(cur, stack, sp, name_abs)?;
                        continue;
                    }
                    if name == b"call" {
                        let body_q = pop(stack, sp).ok_or(TcError { code: 3758, span: name_abs })?;
                        let body_span = match body_q {
                            Value::Quot(s) => s,
                            _ => return Err(TcError { code: 3758, span: name_abs }),
                        };
                        let (qname, qsig, may_suspend) = self.build_quote_word(body_span)?;
                        if may_suspend && !allow_suspend {
                            return Err(TcError { code: 3503, span: name_abs });
                        }

                        let entry = WordEntry {
                            name: TypeAtom::new(b"call").unwrap(),
                            sig: qsig,
                            may_suspend,
                        };
                        apply_sig(stack, sp, &entry, name_abs, self.subtypes)?;
                        let call_sig = self.lir_sig_for_entry(&qsig, name_abs)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::Call {
                                name: qname,
                                sig: call_sig,
                                may_suspend,
                            },
                            name_abs,
                        )?;
                        continue;
                    }
                    if name == b"platform.task.spawn" {
                        let body_q = pop(stack, sp).ok_or(TcError { code: 3754, span: name_abs })?;
                        let body_span = match body_q {
                            Value::Quot(s) => s,
                            _ => return Err(TcError { code: 3754, span: name_abs }),
                        };
                        let (qname, qsig, _may_suspend) = self.build_quote_word(body_span)?;
                        if qsig.in_len != 0 || qsig.out_len != 0 {
                            return Err(TcError { code: 3756, span: name_abs });
                        }

                        let task_ty = TypeAtom::new(b"Task").ok_or(TcError { code: 3757, span: name_abs })?;
                        let task_tid = self.ty_id_of_type(task_ty, name_abs)?;
                        push(stack, sp, Value::Plain(task_ty))?;
                        self.emit_op(
                            cur,
                            lir::OpKind::TaskSpawn {
                                name: qname,
                                task_ty: task_tid,
                            },
                            name_abs,
                        )?;
                        continue;
                    }
                    if name == b"platform.task.run" {
                        if !self.check_no_scoped_live_all(stack, *sp) {
                            return Err(TcError { code: 3502, span: name_abs });
                        }
                        let body_q = pop(stack, sp).ok_or(TcError { code: 3750, span: name_abs })?;
                        let body_span = match body_q {
                            Value::Quot(s) => s,
                            _ => return Err(TcError { code: 3751, span: name_abs }),
                        };
                        let base_stack = *stack;
                        let base_sp = *sp;
                        cur = self.compile_quote_span(cur, stack, sp, body_span, true, false)?;
                        if *sp != base_sp {
                            return Err(TcError { code: 3752, span: name_abs });
                        }
                        for i in 0..base_sp {
                            if stack[i] != base_stack[i] {
                                return Err(TcError { code: 3753, span: name_abs });
                            }
                        }
                        continue;
                    }

                    if let Some(idx) = find_local(&self.locals, self.local_len, TypeAtom::new(name).unwrap_or(TypeAtom::EMPTY)) {
                        if !self.local_live[idx] {
                            return Err(TcError { code: 3514, span: name_abs });
                        }
                        if is_iso_type(self.iso, self.local_tys[idx]) {
                            self.local_live[idx] = false;
                        }
                        if self.local_scoped[idx] != 0 {
                            push(
                                stack,
                                sp,
                                Value::Scoped {
                                    ty: self.local_tys[idx],
                                    scope: self.local_scoped[idx],
                                },
                            )?;
                        } else {
                            push(stack, sp, Value::Plain(self.local_tys[idx]))?;
                        }
                        let tid = self.ty_id_of_type(self.local_tys[idx], name_abs)?;
                        self.emit_op(cur, lir::OpKind::LocalGet { slot: self.local_slot(idx), ty: tid }, name_abs)?;
                        continue;
                    }

                    let entry = lookup(self.env, name).ok_or(TcError { code: 3210, span: name_abs })?;
                    if entry.may_suspend && !allow_suspend {
                        return Err(TcError { code: 3503, span: name_abs });
                    }
                    if entry.may_suspend && !self.check_no_scoped_live_all(stack, *sp) {
                        return Err(TcError { code: 3502, span: name_abs });
                    }
                    apply_sig(stack, sp, entry, name_abs, self.subtypes)?;

                    let builtin = match name {
                        b"+" => Some(lir::OpKind::AddI64),
                        b"-" => Some(lir::OpKind::SubI64),
                        b"*" => Some(lir::OpKind::MulI64),
                        b">" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Gt }),
                        b"<" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Lt }),
                        b">=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Ge }),
                        b"<=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Le }),
                        b"==" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Eq }),
                        b"!=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Ne }),
                        b"and" => Some(lir::OpKind::AndBool),
                        b"or" => Some(lir::OpKind::OrBool),
                        b"not" => Some(lir::OpKind::NotBool),
                        _ => None,
                    };
                    if let Some(kind) = builtin {
                        self.emit_op(cur, kind, name_abs)?;
                    } else {
                        let call_sig = self.lir_sig_for_entry(&entry.sig, name_abs)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::Call {
                                name: lir_atom(name)?,
                                sig: call_sig,
                                may_suspend: entry.may_suspend,
                            },
                            name_abs,
                        )?;
                    }
                }
                _ => {
                }
            }
        }

        Ok(cur)
    }
}
