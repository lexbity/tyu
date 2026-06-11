use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {

    pub(super) fn compile_return(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        name_abs: Span,
    ) -> Result<lir::BlockId, TcError> {
        let want = self.sig.out_len as usize;
        if *sp != want {
            return Err(TcError::ReturnStackDepth { span });
        }
        if self.any_scoped_live(stack, *sp) {
            return Err(TcError::ReturnWithScoped { span });
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
                return Err(TcError::ReturnTypeMismatch { span });
            }
        }
        self.emit_op(cur, lir::OpKind::Ret, name_abs)?;
        Ok(cur)
    }


    pub(super) fn compile_call_quote(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name_abs: Span,
        allow_suspend: bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError::CallPopQuot { span: name_abs })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::CallPopQuot { span: name_abs }),
        };
        let (qname, qsig, performs, qbound) = self.build_quote_word(body_span, observer)?;
        if performs.contains(EffectSet::SUSPEND) && !allow_suspend {
            return Err(TcError::SuspendForbidden { span: name_abs });
        }

        self.acc = self.acc.compose(qbound);
        let entry = WordEntry {
            name: TypeAtom::new(b"call").unwrap(),
            sig: qsig,
            performs,
            requires: CapSet::empty(),
            bound: qbound,
        };
        apply_sig(stack, sp, &entry, name_abs, self.subtypes)?;
        let call_sig = self.lir_sig_for_entry(&qsig, name_abs)?;
        self.emit_op(
            cur,
            lir::OpKind::Call {
                name: qname,
                sig: call_sig,
                performs,
                requires: CapSet::empty(),
                bound: qbound,
            },
            name_abs,
        )?;
        Ok(cur)
    }


    pub(super) fn compile_scoped_block(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
        lex: &mut Lexer,
        allow_suspend: bool,
        allow_locals: bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
        if *sp == 0 {
            return Err(TcError::EmptyStackForScoped { span });
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
            let scope_id = self
                .enter_scope()
                .ok_or(TcError::ScopeDepthExceeded { span })?;
            let slice_ty =
                slice_type_of_elem(elem, mut_scope).ok_or(TcError::SliceTypeFailed { span })?;
            let len = array_len(top_ty).ok_or(TcError::SliceTypeFailed { span })?;
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

            let block = capture_scoped_block(lex, slice, tok.span)
                .map_err(|_| TcError::TypeParseFailed { span })?;
            let block_allow_suspend = if mut_scope { false } else { allow_suspend };
            cur = self.compile_span(
                cur,
                stack,
                sp,
                Span::new(span.start + block.inner_start, span.start + block.inner_end),
                block_allow_suspend,
                allow_locals,
                observer,
            )?;

            if self.stack_has_scope(stack, *sp, scope_id) {
                return Err(TcError::ScopedMarkerLeak { span });
            }
            self.invalidate_scope_locals(scope_id);
            self.leave_scope(scope_id);
        } else if top_ty == TypeAtom::new(b"Region").unwrap() {
            let scope_id = self
                .enter_scope()
                .ok_or(TcError::ScopeDepthExceeded { span })?;
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

            let block = capture_scoped_block(lex, slice, tok.span)
                .map_err(|_| TcError::TypeParseFailed { span })?;
            let block_allow_suspend = if mut_scope { false } else { allow_suspend };
            cur = self.compile_span(
                cur,
                stack,
                sp,
                Span::new(span.start + block.inner_start, span.start + block.inner_end),
                block_allow_suspend,
                allow_locals,
                observer,
            )?;

            if self.stack_has_scope(stack, *sp, scope_id) {
                return Err(TcError::ScopedMarkerLeak { span });
            }
            self.invalidate_scope_locals(scope_id);
            self.leave_scope(scope_id);
        } else {
            return Err(TcError::ScopedTypeMismatch { span });
        }
        Ok(cur)
    }


}
