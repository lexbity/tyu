use super::*;

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
        let next = lex.next();
        let next = if next.kind == TokenKind::EffectSet {
            let tok_bytes = &slice[next.span.start..next.span.end];
            performs = Self::parse_effect_set(tok_bytes);
            lex.next()
        } else {
            next
        };
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
    ) -> Result<(lir::Atom, WordSig, EffectSet), TcError> {
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
                self.resources,
                self.nominals,
                self.iso,
                self.checks,
                self.allow_raw_casts,
                arena,
                sig,
                name,
            )?;

            let mut stack: [Value; 256] = [Value::Plain(TypeAtom::EMPTY); 256];
            let mut sp: usize = 0;
            for i in 0..(sig.in_len as usize) {
                stack[sp] = Value::Plain(sig.inputs[i]);
                sp += 1;
            }
            let cur = lir::BlockId(0);
            let cur = qgen.emit_prologue(cur, &mut stack, &mut sp, None, observer)?;
            let cur = qgen.compile_span(
                cur,
                &mut stack,
                &mut sp,
                parsed.body,
                parsed.performs.contains(EffectSet::SUSPEND),
                false,
                observer,
            )?;
            if !qgen.check_no_scoped_live(&stack, sp) {
                return Err(TcError::ScopedLeak { span: quot_span });
            }
            if !qgen.terminated {
                if sp != sig.out_len as usize {
                    return Err(TcError::OutputCountMismatch { span: quot_span });
                }
                for (i, v) in stack.iter().enumerate().take(sig.out_len as usize) {
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
                    if !type_compatible(got, sig.outputs[i], self.subtypes) {
                        return Err(TcError::OutputTypeMismatch { span: quot_span });
                    }
                }
                qgen.emit_op(cur, lir::OpKind::Ret, quot_span)?;
            }
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
        Ok((name, sig, parsed.performs))
    }
}
