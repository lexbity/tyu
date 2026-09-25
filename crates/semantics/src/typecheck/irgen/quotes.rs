use super::*;
use crate::typecheck::context::{ContextKind, FrameParam};
use crate::typecheck::error::EscapeKind;

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn lir_sig_for_entry(
        &mut self,
        sig: &WordSig,
        span: Span,
    ) -> Result<lir::Sig, TcError> {
        let mut out = lir::Sig::empty();
        out.in_len = sig.in_len;
        out.out_len = sig.out_len;
        for i in 0..(sig.in_len as usize) {
            out.inputs[i] = self.ty_id_of_type(sig.inputs[i], span)?;
        }
        for i in 0..(sig.out_len as usize) {
            out.outputs[i] = self.ty_id_of_type(sig.outputs[i], span)?;
        }
        Ok(out)
    }

    pub(super) fn parse_effect_set(bytes: &[u8]) -> EffectSet {
        if bytes.len() < 4 {
            return EffectSet::empty();
        }
        let needle = b"suspend";
        let has_suspend = bytes.windows(needle.len()).any(|w| w == needle);
        if has_suspend {
            EffectSet::from_bits(EffectSet::SUSPEND)
        } else {
            EffectSet::empty()
        }
    }

    pub(super) fn parse_quote_sig(&self, quot_span: Span) -> Result<QuoteSig, TcError> {
        if quot_span.end <= quot_span.start + 2 {
            return Err(TcError::QuoteSyntax { span: quot_span });
        }
        let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
        let slice = &self.src[inner.start..inner.end];
        let mut lex = Lexer::new(slice);
        let first = lex.next();
        if first.kind != TokenKind::PunctLParen {
            return Err(TcError::QuoteSyntax { span: quot_span });
        }
        let sig = capture_balanced(
            &mut lex,
            slice,
            TokenKind::PunctLParen,
            TokenKind::PunctRParen,
            first.span.start,
        )
        .map_err(|_| TcError::Internal { span: quot_span })?;
        let sig_span = Span::new(inner.start + sig.start, inner.start + sig.end);
        let sig = crate::typecheck::parse::parse_word_sig(self.src, sig_span)
            .map_err(|_| TcError::TypeParseFailed { span: sig_span })?;

        let mut performs = EffectSet::empty();
        let mut has_explicit_performs = false;
        let mut next = lex.next();
        // S-12: `performs {suspend}` replaces old `!{suspend}`.
        if next.kind == TokenKind::KwPerforms {
            has_explicit_performs = true;
            let brace_tok = lex.next();
            if brace_tok.kind == TokenKind::PunctLBrace {
                let mut depth = 1u32;
                let content_start = brace_tok.span.end;
                let mut content_end = brace_tok.span.end;
                while depth > 0 {
                    let t = lex.next();
                    content_end = t.span.end;
                    if t.kind == TokenKind::PunctLBrace {
                        depth += 1;
                    }
                    if t.kind == TokenKind::PunctRBrace {
                        depth -= 1;
                    }
                    if t.kind == TokenKind::Eof {
                        break;
                    }
                }
                #[allow(unused_assignments)]
                let _ = &content_end;
                if content_end > content_start {
                    let inner = &slice[content_start..content_end - 1];
                    let mut buf = [0u8; 128];
                    let mut buf_len = 0usize;
                    buf[buf_len] = b'!';
                    buf_len += 1;
                    buf[buf_len] = b'{';
                    buf_len += 1;
                    for &b in inner.iter().take(125 - buf_len) {
                        buf[buf_len] = b;
                        buf_len += 1;
                    }
                    buf[buf_len] = b'}';
                    buf_len += 1;
                    performs = Self::parse_effect_set(&buf[..buf_len]);
                }
            }
            next = lex.next();
        }
        let body_start = if next.kind == TokenKind::Eof {
            inner.end
        } else {
            inner.start + next.span.start
        };
        let body = Span::new(body_start, inner.end);
        Ok(QuoteSig {
            sig,
            performs,
            requires: CapSet::empty(),
            bound: StackBound::ID,
            body,
            has_explicit_performs,
        })
    }

    pub(super) fn quote_word_name(&mut self) -> lir::Atom {
        let id = self.quote_id;
        self.quote_id = id.wrapping_add(1);
        let mut buf = [0u8; 16];
        let mut i = 0usize;
        buf[i] = b'_';
        i += 1;
        buf[i] = b'_';
        i += 1;
        buf[i] = b'q';
        i += 1;
        buf[i] = b'u';
        i += 1;
        buf[i] = b'o';
        i += 1;
        buf[i] = b't';
        i += 1;
        buf[i] = b'_';
        i += 1;
        let mut v = id;
        for _ in 0..8 {
            let digit = (v & 0xf) as u8;
            let b = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + (digit - 10)
            };
            buf[i] = b;
            i += 1;
            v >>= 4;
        }
        lir::Atom::new(&buf[..i]).unwrap_or(lir::AT_QUOT)
    }

    pub(super) fn build_quote_word(
        &mut self,
        quot_span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<(lir::Atom, WordSig, EffectSet, StackBound), TcError> {
        self.build_quote_word_with_context(quot_span, false, observer)
    }

    /// Build a quotation word, optionally inheriting the parent's lock state
    /// and borrow ledger (used by `call` via `compile_call_quote`).
    pub(super) fn build_quote_word_with_context(
        &mut self,
        quot_span: Span,
        inherit_context: bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<(lir::Atom, WordSig, EffectSet, StackBound), TcError> {
        let parsed = self.parse_quote_sig(quot_span)?;
        let name = self.quote_word_name();
        let sig = parsed.sig;

        let word = {
            let arena = unsafe { &mut *self.arena };
            let mut qgen = IrWordGen::new(
                self.src,
                self.env,
                self.subtypes,
                self.mmio,
                self.descriptor,
                self.resources,
                self.nominals,
                self.iso,
                self.checks,
                self.allow_raw_casts,
                // Quote words compile with extraction disabled (P2): their
                // internal `_quot_*` names are per-word-local, so obligation
                // ids keyed on them would collide across words. Their subtype
                // sites are covered by P5's interval engine under the caller
                // word's context.
                None,
                // No verdicts either: quote emissions are inherited from the
                // caller (P4 keeps quote checks, conservative).
                None,
                false,
                arena,
                sig,
                name,
            )?;

            if inherit_context {
                // S4: propagate the parent's lock context to the quotation.
                if let Some(lf) = self.ctx.lock_frame() {
                    let resource = lf.resource();
                    let param = resource.map_or(FrameParam::None, FrameParam::Resource);
                    qgen.ctx.push(ContextKind::Lock, param, Span::new(0, 0))?;
                }
            }

            let mut stack: [Value; 256] = [Value::Plain(TypeAtom::EMPTY); 256];
            let mut sp: usize = 0;
            for i in 0..(sig.in_len as usize) {
                stack[sp] = Value::Plain(sig.inputs[i]);
                sp += 1;
            }
            let cur = lir::BlockId(0);
            let cur = qgen.emit_prologue(cur, &mut stack, &mut sp, None, observer)?;
            // Quotation words are self-contained: their suspend permission
            // comes from the parsed annotation, not the parent's context.
            // We compile a fresh IrWordGen (with its own empty context stack),
            // so the suspend blocker uses the quotation's declared performs.
            // We push WordBody if the quotation annotation grants SUSPENDABLE.
            if parsed.performs.contains(EffectSet::SUSPEND) {
                qgen.ctx
                    .push(ContextKind::WordBody, FrameParam::None, parsed.body)?;
            }
            let cur = qgen.compile_span(cur, &mut stack, &mut sp, parsed.body, false, observer)?;
            while qgen.ctx.depth() > 0 {
                qgen.ctx.pop();
            }

            // S8: E5005 for quotation annotations.
            if parsed.has_explicit_performs {
                let missing = qgen.word.performs.minus(parsed.performs);
                if !missing.is_empty() {
                    return Err(TcError::EffectNotDeclared { span: quot_span });
                }
            }

            if !qgen.check_no_scoped_live(&stack, sp) {
                return Err(TcError::BorrowEscape {
                    span: quot_span,
                    kind: EscapeKind::AtClose,
                });
            }
            if !qgen.terminated {
                if sp != sig.out_len as usize {
                    return Err(TcError::OutputCountMismatch { span: quot_span });
                }
                for (i, v) in stack.iter().enumerate().take(sig.out_len as usize) {
                    let got = v.to_type_atom();
                    if !type_compatible(got, sig.outputs[i], self.subtypes) {
                        return Err(TcError::OutputTypeMismatch { span: quot_span });
                    }
                }
                qgen.emit_op(cur, lir::OpKind::Ret, quot_span)?;
            }
            qgen.fill_subtype_bases(quot_span)?;
            qgen.word.bound = qgen.acc;
            qgen.word
        };

        let word = unsafe {
            let arena = &mut *self.arena;
            let w = arena.alloc(word, quot_span)?;
            &*(w as *const lir::Word)
        };
        let ew: *mut FixedVec<&'r lir::Word, { arena::QUOTE_WORD_CAP }> = &mut self.extra_words;
        unsafe {
            (*ew)
                .push(word)
                .map_err(|_| TcError::OpTableFull { span: quot_span })?;
        }
        Ok((name, sig, parsed.performs, word.bound))
    }
}
