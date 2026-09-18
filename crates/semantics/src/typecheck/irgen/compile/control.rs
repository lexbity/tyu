use super::*;
use crate::typecheck::context::FrameParam;
use crate::typecheck::error::EscapeKind;

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
            return Err(TcError::BorrowEscape {
                span,
                kind: EscapeKind::AtReturn,
            });
        }
        for (i, v) in stack.iter().enumerate().take(want) {
            let got = v.to_type_atom();
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
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError::CallPopQuot { span: name_abs })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::CallPopQuot { span: name_abs }),
        };
        // `call` consumes the quotation value at runtime (BUG-012).
        self.acc = self.acc.compose(StackBound {
            net: -1,
            high: High::Slots(0),
        });
        // Build the runtime word first (with parent's lock context so
        // resource access inside the call works correctly).
        let (qname, qsig, performs, qbound) =
            self.build_quote_word_with_context(body_span, true, observer)?;
        if performs.contains(EffectSet::SUSPEND) {
            if let Some(blocker) = self.suspend_blocker(stack, *sp) {
                let span = match blocker {
                    SuspendBlocker::Frame(s) => s,
                    SuspendBlocker::BorrowLive { frame_span, .. } => frame_span,
                    SuspendBlocker::Undeclared => name_abs,
                };
                return Err(TcError::SuspendForbidden { span });
            }
        }

        // S-7: call shares the live ledger.  Scan the parent's stack for
        // borrows that would conflict with borrows in the call's quotation.
        // For each live Ptr on the parent's stack, check if the quotation's
        // word contains an AddrOf op with the same place root.
        for v in stack[..*sp].iter() {
            if let Value::Ptr {
                place, mutable: pm, ..
            } = *v
            {
                if place == PLACE_NONE {
                    continue;
                }
                let idx = place.0 as usize;
                if idx >= self.ledger_len as usize {
                    continue;
                }
                // Conservative check: if the parent has a MUTABLE borrow
                // live, reject the call.  This is stricter than needed but
                // sound.  A precise check would scan the quotation's ops
                // for AddrOf with matching place.
                if pm {
                    return Err(TcError::BorrowAlias {
                        span: name_abs,
                        first: self.ledger[idx].origin,
                    });
                }
            }
        }
        if performs.contains(EffectSet::SUSPEND) {
            if let Some(blocker) = self.suspend_blocker(stack, *sp) {
                let span = match blocker {
                    SuspendBlocker::Frame(s) => s,
                    SuspendBlocker::BorrowLive { frame_span, .. } => frame_span,
                    SuspendBlocker::Undeclared => name_abs,
                };
                return Err(TcError::SuspendForbidden { span });
            }
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
        allow_locals: bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
        if *sp == 0 {
            return Err(TcError::EmptyStackForScoped { span });
        }
        let top_ty = stack[*sp - 1].to_type_atom();
        // Compute per-kind pair (scoped_ty, enter_len) up front, then share
        // the rest of the body (DEBT-3 unification).
        let (scoped_ty, enter_len) = if let Some(elem) = array_elem_type(top_ty) {
            let slice_ty =
                slice_type_of_elem(elem, mut_scope).ok_or(TcError::SliceTypeFailed { span })?;
            let len = array_len(top_ty).ok_or(TcError::SliceTypeFailed { span })?;
            (slice_ty, len)
        } else if top_ty == TypeAtom::new(b"Region").unwrap() {
            (region_ref_type(mut_scope), 0)
        } else {
            return Err(TcError::ScopedTypeMismatch { span });
        };

        let scope_id = self
            .enter_scope()
            .ok_or(TcError::ScopeDepthExceeded { span })?;
        push(
            stack,
            sp,
            Value::Scoped {
                ty: scoped_ty,
                scope: scope_id,
            },
        )?;
        let tid = self.ty_id_of_type(scoped_ty, span)?;
        self.emit_op(
            cur,
            lir::OpKind::ScopedEnter {
                ty: tid,
                len: enter_len,
            },
            Span::new(span.start + tok.span.start, span.start + tok.span.end),
        )?;

        let block = capture_scoped_block(lex, slice, tok.span)
            .map_err(|_| TcError::TypeParseFailed { span })?;

        // Push context frame: MutBorrow forbids SUSPEND, ReadBorrow is transparent.
        let ctx_kind = if mut_scope {
            ContextKind::MutBorrow
        } else {
            ContextKind::ReadBorrow
        };
        self.ctx.push(
            ctx_kind,
            FrameParam::Scope(scope_id),
            Span::new(span.start + tok.span.start, span.start + tok.span.end),
        )?;
        cur = self.compile_span(
            cur,
            stack,
            sp,
            Span::new(span.start + block.inner_start, span.start + block.inner_end),
            allow_locals,
            observer,
        )?;
        self.ctx.pop();

        // S6: scope-close classification — E5020 BorrowEscape if anything
        // still carries this scope, THEN invalidate (R-3 ordering).
        self.close_scope(stack, *sp, scope_id, span)?;
        self.invalidate_scope_locals(scope_id);
        self.leave_scope(scope_id);
        Ok(cur)
    }
}
