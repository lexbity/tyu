use super::super::SuspendBlocker;
use super::*;
use crate::typecheck::error::EscapeKind;
use crate::typecheck::value::PLACE_NONE;

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn compile_name(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        lex: &mut Lexer,
        tok: Token,
        name: &[u8],
        name_abs: Span,
        _allow_locals: bool,
        terminated: &mut bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        if name == b"true" || name == b"false" {
            self.compile_bool_literal(cur, stack, sp, name, name_abs)?;
            return Ok(cur);
        }

        if tok.kind == TokenKind::Ident {
            if let Some(atom) = TypeAtom::new(name) {
                if resource_ty(self.resources, atom).is_some() {
                    push(stack, sp, Value::Resource(atom))?;
                    return Ok(cur);
                }
            }
        }

        if tok.kind == TokenKind::Ident {
            if let Some(dot) = name.iter().position(|&b| b == b'.') {
                if dot + 1 < name.len() && !name[dot + 1..].contains(&b'.') {
                    let enum_part = &name[..dot];
                    let var_part = &name[dot + 1..];
                    if let (Some(enum_ty), Some(var)) =
                        (TypeAtom::new(enum_part), TypeAtom::new(var_part))
                    {
                        if self.compile_enum_variant(cur, stack, sp, name_abs, enum_ty, var)? {
                            return Ok(cur);
                        }
                    }
                }
            }
        }

        if !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
            return self.compile_load_store(cur, stack, sp, name_abs, name, tok);
        }

        if tok.kind == TokenKind::Ident {
            let place = crate::typecheck::mmio::qualname_to_placepath(name, name_abs);
            if let Some(res) = resolve_mmio_place(self.mmio, self.src, &place, name_abs)? {
                let (window, offset, access) = match res {
                    MmioResolved::Reg(reg) => (reg.window, reg.offset, reg.access),
                    MmioResolved::Field(field) => (field.window, field.offset, field.reg_access),
                };
                self.record_window(window, access, name_abs)?;
                push(stack, sp, Value::MmioPlace(res))?;
                self.emit_op(
                    cur,
                    lir::OpKind::MmioPlace {
                        place: lir_atom(name)?,
                        window,
                        offset,
                    },
                    name_abs,
                )?;
                return Ok(cur);
            }
        }

        if name == b"dup" || name == b"drop" || name == b"swap" {
            return self.compile_stack_op(cur, stack, sp, span, name_abs, name);
        }

        if name == b"as" || name == b"as?" || name == b"bitcast" {
            return self.compile_cast(cur, stack, sp, span, slice, lex, name, name_abs);
        }

        if name == b"if" {
            cur = self.compile_if(cur, stack, sp, name_abs, observer)?;
            return Ok(cur);
        }
        if name == b"while" {
            cur = self.compile_while(cur, stack, sp, name_abs, observer)?;
            return Ok(cur);
        }
        if name == b"loop" {
            cur = self.compile_loop(cur, stack, sp, name_abs, observer)?;
            return Ok(cur);
        }

        if name == b"return" {
            *terminated = true;
            self.terminated = true;
            return self.compile_return(cur, stack, sp, span, name_abs);
        }

        if name == b"lock" {
            let mut probe = *lex;
            let next = probe.next();
            if next.kind == TokenKind::PunctLBracket {
                *lex = probe;
                let _block = capture_scoped_block(lex, slice, next.span)
                    .map_err(|_| TcError::TypeParseFailed { span: name_abs })?;
                let full_span = Span::new(span.start + next.span.start, span.start + lex.pos());
                push(stack, sp, Value::Quot(full_span))?;
            }
            cur = self.compile_lock(cur, stack, sp, name_abs, observer)?;
            return Ok(cur);
        }

        if name == b"call" {
            return self.compile_call_quote(cur, stack, sp, name_abs, observer);
        }

        if name == b"platform.task.spawn" {
            return self.compile_task_spawn(cur, stack, sp, name_abs, observer);
        }

        if name == b"platform.task.run" {
            return self.compile_task_run(cur, stack, sp, name_abs, span, observer);
        }

        if let Some(idx) = find_local(
            &self.locals,
            self.local_len,
            TypeAtom::new(name).unwrap_or(TypeAtom::EMPTY),
        ) {
            return self.compile_local_ref(cur, stack, sp, name_abs, idx);
        }

        self.compile_env_word(cur, stack, sp, name, name_abs)?;
        Ok(cur)
    }

    pub(super) fn compile_local_ref(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name_abs: Span,
        idx: usize,
    ) -> Result<lir::BlockId, TcError> {
        if !self.local_live[idx] {
            // Iso types are consumed (moved) on first use.  A second reference
            // is a use-after-move; non-iso locals are just not live.
            if is_iso_type(self.iso, self.local_tys[idx]) {
                return Err(TcError::IsoUseAfterMove { span: name_abs });
            }
            // If this is a borrow-typed local that was already consumed,
            // reject with E5023 (reuse of linear borrow local).
            if self.local_place[idx] != PLACE_NONE {
                let origin = if (self.ledger_len as usize) > self.local_place[idx].0 as usize {
                    self.ledger[self.local_place[idx].0 as usize].origin
                } else {
                    name_abs
                };
                return Err(TcError::BorrowLocalReuse {
                    span: name_abs,
                    first: origin,
                });
            }
            return Err(TcError::LocalNotLive { span: name_abs });
        }

        // Linear borrow-typed locals (PTR_MUT): consume on first ref.
        let is_mut_borrow =
            self.local_tys[idx] == TypeAtom::PTR_MUT && self.local_place[idx] != PLACE_NONE;
        if is_mut_borrow {
            self.local_live[idx] = false;
        }
        if is_iso_type(self.iso, self.local_tys[idx]) {
            self.local_live[idx] = false;
        }

        // Push the value — for borrow locals, reconstruct Value::Ptr.
        if self.local_place[idx] != PLACE_NONE {
            let mutable = self.local_tys[idx] == TypeAtom::PTR_MUT;
            push(
                stack,
                sp,
                Value::Ptr {
                    ty: self.local_tys[idx],
                    mutable,
                    place: self.local_place[idx],
                },
            )?;
        } else if matches!(self.local_tys[idx], TypeAtom::PTR | TypeAtom::PTR_MUT) {
            // Raw pointer local (no provenance): reconstruct the Value::Ptr
            // shape so the typed load/store surface (@TY / !TY) accepts it
            // (BUG-003).  Pointee is unknown (EMPTY), so the asserted type
            // wins — identical to the `as ptr` cast shape.
            let mutable = self.local_tys[idx] == TypeAtom::PTR_MUT;
            push(
                stack,
                sp,
                Value::Ptr {
                    ty: TypeAtom::EMPTY,
                    mutable,
                    place: PLACE_NONE,
                },
            )?;
        } else if self.local_scoped[idx] != 0 {
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
        self.emit_op(
            cur,
            lir::OpKind::LocalGet {
                slot: self.local_slot(idx),
                ty: tid,
            },
            name_abs,
        )?;
        Ok(cur)
    }

    pub(super) fn compile_env_word(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name: &[u8],
        name_abs: Span,
    ) -> Result<(), TcError> {
        let entry = lookup(self.env, name).ok_or(TcError::WordNotFound { span: name_abs })?;
        if entry.performs.contains(EffectSet::SUSPEND) {
            if let Some(blocker) = self.suspend_blocker(stack, *sp) {
                let span = match blocker {
                    SuspendBlocker::Frame(s) => s,
                    SuspendBlocker::BorrowLive { frame_span, .. } => frame_span,
                    SuspendBlocker::Undeclared => name_abs,
                };
                return Err(TcError::SuspendForbidden { span });
            }
        }
        apply_sig(stack, sp, entry, name_abs, self.subtypes)?;

        // S7: accumulate callee effects into the word's computed performs set.
        self.word.performs = self.word.performs.union(entry.performs);

        // S8: self-recursive call → DIVERGE.
        if name == self.word.name.as_bytes() {
            self.word.performs = self
                .word
                .performs
                .union(EffectSet::from_bits(EffectSet::DIVERGE));
            self.acc = self.acc.compose(StackBound {
                net: 0,
                high: High::Top,
            });
            // S9: 5040 — self-recursion in a bounded context.
            if self.ctx.ambient_forbids.contains(EffectSet::DIVERGE) {
                return Err(TcError::DivergeInBounded { span: name_abs });
            }
        }

        // S9: 5040 — callee performs DIVERGE in a bounded context.
        if entry.performs.contains(EffectSet::DIVERGE)
            && self.ctx.ambient_forbids.contains(EffectSet::DIVERGE)
        {
            return Err(TcError::DivergeInBounded { span: name_abs });
        }

        let builtin = match name {
            b"+" => Some(lir::OpKind::AddI64),
            b"-" => Some(lir::OpKind::SubI64),
            b"*" => Some(lir::OpKind::MulI64),
            b">" => Some(lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Gt,
            }),
            b"<" => Some(lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Lt,
            }),
            b">=" => Some(lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Ge,
            }),
            b"<=" => Some(lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Le,
            }),
            b"==" => Some(lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Eq,
            }),
            b"!=" => Some(lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Ne,
            }),
            b"and" => Some(lir::OpKind::AndBool),
            b"or" => Some(lir::OpKind::OrBool),
            b"not" => Some(lir::OpKind::NotBool),
            _ => None,
        };
        if let Some(kind) = builtin {
            self.emit_op(cur, kind, name_abs)?;
        } else {
            // Real call: `emit_op` does not account `Call`, so compose the
            // caller-visible delta from the callee's signature (`out − in`).
            // `entry.bound`'s `net` is unreliable for imported/stub words
            // (net 0 regardless of sig) and must not drive branch-merging
            // equality (BUG-012); its `high` still carries the callee's peak.
            self.acc = self.acc.compose(StackBound {
                net: entry.sig.out_len as i16 - entry.sig.in_len as i16,
                high: entry.bound.high,
            });
            let call_sig = self.lir_sig_for_entry(&entry.sig, name_abs)?;
            self.emit_op(
                cur,
                lir::OpKind::Call {
                    name: lir_atom(name)?,
                    sig: call_sig,
                    performs: entry.performs,
                    requires: entry.requires,
                    bound: entry.bound,
                },
                name_abs,
            )?;
        }
        Ok(())
    }

    pub(super) fn compile_cast(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        lex: &mut Lexer,
        name: &[u8],
        name_abs: Span,
    ) -> Result<lir::BlockId, TcError> {
        let first = lex.next();
        let start = first.span.start;
        let (to_ty, next) = crate::typecheck::parse::parse_type_expr(slice, start).ok_or(
            TcError::CastParseFailed {
                span: Span::new(span.start + first.span.start, span.start + first.span.end),
            },
        )?;
        lex.set_pos(next);
        let v = pop(stack, sp).ok_or(TcError::CastPopValue { span })?;
        let from_ty = v.to_type_atom();
        let subtype = find_subtype(self.subtypes, to_ty);
        if let Some(st) = subtype {
            if !type_compatible(from_ty, st.base, self.subtypes) {
                return Err(TcError::CastSubtypeMismatch { span });
            }
        }

        if name == b"bitcast" {
            if subtype.is_some() {
                return Err(TcError::BitcastOnSubtype { span: name_abs });
            }
            let Some((from_bits, _)) = self.ty_bits_signed(from_ty) else {
                return Err(TcError::BitcastWidthUnknown { span: name_abs });
            };
            let Some((to_bits, _)) = self.ty_bits_signed(to_ty) else {
                return Err(TcError::BitcastWidthUnknown { span: name_abs });
            };
            if from_bits != to_bits {
                return Err(TcError::BitcastWidthMismatch { span: name_abs });
            }
            if !self.check_raw_cast_allowed(from_ty, to_ty) {
                return Err(TcError::CastNotAllowed { span: name_abs });
            }
        } else {
            if !self.check_raw_cast_allowed(from_ty, to_ty) {
                return Err(TcError::CastNotAllowed { span: name_abs });
            }
        }

        let from_id = self.ty_id_of_type(from_ty, name_abs)?;
        let to_id = self.ty_id_of_type(to_ty, name_abs)?;
        if name == b"bitcast" {
            self.emit_op(
                cur,
                lir::OpKind::Bitcast {
                    from: from_id,
                    to: to_id,
                },
                name_abs,
            )?;
        } else {
            self.emit_op(
                cur,
                lir::OpKind::Cast {
                    from: from_id,
                    to: to_id,
                },
                name_abs,
            )?;
        }
        // S-8: raw pointer casts (as ptr / as ptr_mut) produce Value::Ptr
        // with PLACE_NONE (untyped — the provenance is lost in the cast).
        let push_val = if to_ty == TypeAtom::PTR || to_ty == TypeAtom::PTR_MUT {
            Value::Ptr {
                ty: TypeAtom::EMPTY,
                mutable: to_ty == TypeAtom::PTR_MUT,
                place: PLACE_NONE,
            }
        } else {
            Value::Plain(to_ty)
        };
        if name == b"as?" {
            push(stack, sp, push_val)?;
            if let Some(st) = subtype {
                let tmp = self.temp_base_slot();
                self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                self.emit_op(
                    cur,
                    lir::OpKind::LocalSet {
                        slot: tmp,
                        ty: to_id,
                    },
                    name_abs,
                )?;

                self.emit_op(
                    cur,
                    lir::OpKind::LocalGet {
                        slot: tmp,
                        ty: to_id,
                    },
                    name_abs,
                )?;
                self.emit_op(cur, lir::OpKind::ConstI64(st.min), name_abs)?;
                self.emit_op(
                    cur,
                    lir::OpKind::Cmp {
                        out: lir::TY_BOOL,
                        kind: lir::CmpKind::Ge,
                    },
                    name_abs,
                )?;
                self.emit_op(
                    cur,
                    lir::OpKind::LocalGet {
                        slot: tmp,
                        ty: to_id,
                    },
                    name_abs,
                )?;
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
            return Ok(cur);
        }
        push(stack, sp, push_val)?;
        if name == b"as" {
            if let Some(st) = subtype {
                if self.checks == ChecksMode::All {
                    let tmp = self.temp_base_slot();
                    self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                    self.emit_op(
                        cur,
                        lir::OpKind::LocalSet {
                            slot: tmp,
                            ty: to_id,
                        },
                        name_abs,
                    )?;
                    self.emit_subtype_range_trap(cur, tmp, to_id, &st, name_abs)?;
                }
            }
        }
        Ok(cur)
    }

    pub(super) fn compile_stack_op(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        name_abs: Span,
        name: &[u8],
    ) -> Result<lir::BlockId, TcError> {
        if name == b"dup" {
            let top = pop(stack, sp).ok_or(TcError::StackUnderflow { span })?;
            let top_ty = top.to_type_atom();
            // Reject dup of mutable borrow (E5022) before iso check.
            if matches!(top, Value::Ptr { mutable: true, place: p, .. } if p != PLACE_NONE)
                || matches!(top, Value::MmioPtr { mutable: true, .. })
            {
                return Err(TcError::BorrowDupMut {
                    span: name_abs,
                    first: name_abs,
                });
            }
            if is_iso_type(self.iso, top_ty) {
                return Err(TcError::IsoDup { span: name_abs });
            }
            push(stack, sp, top)?;
            push(stack, sp, top)?;
            let tid = self.ty_id_of_value(top, name_abs)?;
            self.emit_op(cur, lir::OpKind::Dup { ty: tid }, name_abs)?;
            return Ok(cur);
        }
        if name == b"drop" {
            let top = pop(stack, sp).ok_or(TcError::StackUnderflow { span })?;
            let top_ty = top.to_type_atom();
            if is_iso_type(self.iso, top_ty) {
                return Err(TcError::IsoDrop { span: name_abs });
            }
            let tid = self.ty_id_of_value(top, name_abs)?;
            self.emit_op(cur, lir::OpKind::Drop { ty: tid }, name_abs)?;
            return Ok(cur);
        }
        if name == b"swap" {
            let b = pop(stack, sp).ok_or(TcError::StackUnderflow { span })?;
            let a = pop(stack, sp).ok_or(TcError::StackUnderflow { span })?;
            push(stack, sp, b)?;
            push(stack, sp, a)?;
            let a_id = self.ty_id_of_value(a, name_abs)?;
            let b_id = self.ty_id_of_value(b, name_abs)?;
            self.emit_op(cur, lir::OpKind::Swap { a: a_id, b: b_id }, name_abs)?;
            return Ok(cur);
        }
        Ok(cur)
    }

    pub(super) fn compile_destruct_bind(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
        lex: &mut Lexer,
        allow_locals: bool,
    ) -> Result<lir::BlockId, TcError> {
        if !allow_locals {
            return Err(TcError::BindNotAllowed { span });
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
                            return Err(TcError::ExpectedIdent {
                                span: Span::new(
                                    span.start + name_tok.span.start,
                                    span.start + name_tok.span.end,
                                ),
                            });
                        }
                        let lname = TypeAtom::new(&slice[name_tok.span.start..name_tok.span.end])
                            .ok_or(TcError::TypeParseFailed {
                            span: Span::new(
                                span.start + name_tok.span.start,
                                span.start + name_tok.span.end,
                            ),
                        })?;
                        binds
                            .push(DestructBind {
                                name: lname,
                                borrow: true,
                                mutable: b.kind == TokenKind::PunctAmpBang,
                                span: Span::new(
                                    span.start + name_tok.span.start,
                                    span.start + name_tok.span.end,
                                ),
                            })
                            .map_err(|_| TcError::BindingCapacityExceeded { span })?;
                    }
                    TokenKind::Ident => {
                        if saw_borrow {
                            return Err(TcError::DestructBorrowMix {
                                span: Span::new(span.start + b.span.start, span.start + b.span.end),
                            });
                        }
                        let lname = TypeAtom::new(&slice[b.span.start..b.span.end]).ok_or(
                            TcError::TypeParseFailed {
                                span: Span::new(span.start + b.span.start, span.start + b.span.end),
                            },
                        )?;
                        binds
                            .push(DestructBind {
                                name: lname,
                                borrow: false,
                                mutable: false,
                                span: Span::new(span.start + b.span.start, span.start + b.span.end),
                            })
                            .map_err(|_| TcError::BindingCapacityExceeded { span })?;
                    }
                    _ => {
                        return Err(TcError::DestructExpectedIdent {
                            span: Span::new(span.start + b.span.start, span.start + b.span.end),
                        });
                    }
                }
            }
            if binds.is_empty() {
                return Err(TcError::DestructEmpty { span });
            }
            let base = *stack.get(*sp - 1).ok_or(TcError::StackUnderflow { span })?;
            let has_borrow = binds.iter().any(|b| b.borrow);
            let base_place = match base {
                Value::Ptr { place, .. } => place,
                _ => PLACE_NONE,
            };
            let (struct_ty, base_mut, _base_is_value) = match base {
                Value::Ptr { ty, mutable, .. } => (ty, mutable, false),
                Value::Plain(t) if !has_borrow => (t, false, true),
                _ => return Err(TcError::DestructNotStruct { span }),
            };
            let Some(sinfo) = self.nominals.structs.iter().find(|s| s.name == struct_ty) else {
                return Err(TcError::FieldNotFound { span });
            };
            if binds.len() != sinfo.fields.len() {
                return Err(TcError::DestructFieldCount { span });
            }
            let base_tid = if base_mut {
                lir::TY_PTR_MUT
            } else {
                lir::TY_PTR
            };
            let tmp = self.temp_base_slot();
            self.emit_op(
                cur,
                lir::OpKind::LocalSet {
                    slot: tmp,
                    ty: base_tid,
                },
                Span::new(span.start + tok.span.start, span.start + tok.span.end),
            )?;
            let _ = pop(stack, sp);

            let mut offset: u32 = 0;
            for (idx, field) in sinfo.fields.iter().enumerate() {
                let bind = binds
                    .get(idx)
                    .expect("len verified equal to sinfo.fields.len()");
                if bind.borrow && bind.mutable && !base_mut {
                    return Err(TcError::MutRefToLocal { span: bind.span });
                }
                let fsize = type_size_bytes(field.ty, self.nominals)
                    .ok_or(TcError::FieldSizeError { span: bind.span })?;
                let falign = field_align(fsize);
                offset = align_up(offset, falign);
                let field_offset = offset;
                offset = offset
                    .checked_add(fsize)
                    .ok_or(TcError::FieldSizeError { span: bind.span })?;

                self.emit_op(
                    cur,
                    lir::OpKind::LocalGet {
                        slot: tmp,
                        ty: base_tid,
                    },
                    bind.span,
                )?;
                push(
                    stack,
                    sp,
                    Value::Ptr {
                        ty: struct_ty,
                        mutable: base_mut,
                        place: base_place,
                    },
                )?;
                self.emit_op(
                    cur,
                    lir::OpKind::PtrAddConst {
                        ty: base_tid,
                        offset: field_offset,
                    },
                    bind.span,
                )?;

                if bind.borrow {
                    let ptr_ty = if bind.mutable {
                        TypeAtom::PTR_MUT
                    } else {
                        TypeAtom::PTR
                    };
                    if find_local(&self.locals, self.local_len, bind.name).is_some() {
                        return Err(TcError::BindingAlreadyDefined { span: bind.span });
                    }
                    let slot = self.local_slot(self.local_len);
                    self.locals[self.local_len] = bind.name;
                    self.local_tys[self.local_len] = ptr_ty;
                    self.local_live[self.local_len] = true;
                    self.local_scoped[self.local_len] = 0u16;
                    self.local_place[self.local_len] = base_place;
                    self.local_len += 1;
                    let tid = if bind.mutable {
                        lir::TY_PTR_MUT
                    } else {
                        lir::TY_PTR
                    };
                    let _ = pop(stack, sp);
                    self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, bind.span)?;
                } else {
                    let tid = self.ty_id_of_type(field.ty, bind.span)?;
                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, bind.span)?;
                    let _ = pop(stack, sp);
                    push(stack, sp, Value::Plain(field.ty))?;
                    if find_local(&self.locals, self.local_len, bind.name).is_some() {
                        return Err(TcError::BindingAlreadyDefined { span: bind.span });
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
                return Err(TcError::ExpectedIdent {
                    span: Span::new(span.start + name.span.start, span.start + name.span.end),
                });
            }
            let v = pop(stack, sp).ok_or(TcError::StackUnderflow {
                span: Span::new(span.start + tok.span.start, span.start + tok.span.end),
            })?;
            if v == Value::Plain(TypeAtom::SCOPED) {
                return Err(TcError::BorrowEscape {
                    span,
                    kind: EscapeKind::AtClose,
                });
            }
            let ty = v.to_type_atom();
            let lname = TypeAtom::new(&slice[name.span.start..name.span.end]).ok_or(
                TcError::TypeParseFailed {
                    span: Span::new(span.start + name.span.start, span.start + name.span.end),
                },
            )?;
            if find_local(&self.locals, self.local_len, lname).is_some() {
                return Err(TcError::BindingAlreadyDefined {
                    span: Span::new(span.start + name.span.start, span.start + name.span.end),
                });
            }
            if self.local_len >= self.locals.len() {
                return Err(TcError::BindingCapacityExceeded { span });
            }
            let slot = self.local_slot(self.local_len);
            self.locals[self.local_len] = lname;
            self.local_tys[self.local_len] = ty;
            self.local_live[self.local_len] = true;
            self.local_scoped[self.local_len] = match v {
                Value::Scoped { scope, .. } => scope,
                _ => 0u16,
            };
            self.local_place[self.local_len] = match v {
                Value::Ptr { place, .. } => place,
                _ => PLACE_NONE,
            };
            self.local_len += 1;
            let tid = self.ty_id_of_type(ty, span)?;
            self.emit_op(
                cur,
                lir::OpKind::LocalSet { slot, ty: tid },
                Span::new(span.start + tok.span.start, span.start + tok.span.end),
            )?;
        }
        Ok(cur)
    }

    pub(super) fn compile_quotation(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
        lex: &mut Lexer,
    ) -> Result<lir::BlockId, TcError> {
        let q = capture_balanced(
            lex,
            slice,
            TokenKind::PunctLBracket,
            TokenKind::PunctRBracket,
            tok.span.start,
        )
        .map_err(|_| TcError::TypeParseFailed {
            span: Span::new(span.start + tok.span.start, span.start + tok.span.end),
        })?;
        let q_span = Span::new(span.start + q.start, span.start + q.end);
        push(stack, sp, Value::Quot(q_span))?;
        Ok(cur)
    }
}
