use super::*;

mod literals;
mod memory;
pub(super) mod borrow;
mod channels;
mod control;
mod tasks;
mod names;

impl<'a, 'r> IrWordGen<'a, 'r> {

    pub(super) fn compile_quote_span(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        quot_span: Span,
        allow_locals: bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        if quot_span.end <= quot_span.start + 2 {
            return Ok(cur);
        }
        let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
        self.compile_span(cur, stack, sp, inner, allow_locals, observer)
    }

    pub(super) fn compile_span(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        allow_locals: bool,
        observer: &mut dyn TypecheckObserver,
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

            // D-15: check for number-dot-number (reserved float literal).
            // This check must consume the PunctDot + Number from the lexer
            // before falling through to the normal dispatch.
            if tok.kind == TokenKind::Number {
                let mut probe = lex;
                let dot = probe.next();
                if dot.kind == TokenKind::PunctDot {
                    let num2 = probe.next();
                    if num2.kind == TokenKind::Number {
                        let float_span = Span::new(
                            span.start + tok.span.start,
                            span.start + num2.span.end,
                        );
                        return Err(TcError::FloatSyntax { span: float_span });
                    }
                }
            }

            cur = match tok.kind {
                TokenKind::Number => self.compile_number(cur, stack, sp, span, slice, tok)?,
                TokenKind::String => self.compile_string(cur, stack, sp, span, slice, tok)?,
                TokenKind::PunctArrowBind => self.compile_destruct_bind(
                    cur,
                    stack,
                    sp,
                    span,
                    slice,
                    tok,
                    &mut lex,
                    allow_locals,
                )?,
                TokenKind::PunctLBracket => {
                    self.compile_quotation(cur, stack, sp, span, slice, tok, &mut lex)?
                }
                // S-14: `->` is deleted; `.` auto-projects through pointers.
                TokenKind::PunctArrow => {
                    // Migration hint: use `.` instead of `->`.
                    return Err(TcError::Internal { span: Span::new(span.start + tok.span.start, span.start + tok.span.end) });
                }
                TokenKind::PunctDot => {
                    // `.` handles field access, static index, and dynamic index
                    // with auto-projection (value-extract vs pointer-address).
                    // Delegate to a unified handler.
                    cur = self.compile_dot_op(cur, stack, sp, span, slice, tok, &mut lex)?;
                    cur
                }
                TokenKind::PunctApostrophe => {
                    // S-14: `'` is type-only.  In term position it's a migration hint.
                    return Err(TcError::Internal { span: Span::new(span.start + tok.span.start, span.start + tok.span.end) });
                }
                TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                    self.compile_addr_of(cur, stack, sp, span, slice, tok, &mut lex)?
                }
                TokenKind::PunctPipeGreater => {
                    self.compile_channel_send(cur, stack, sp, span, tok)?
                }
                TokenKind::PunctLessPipe => self.compile_channel_recv(cur, stack, sp, span, tok)?,
                TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => self
                    .compile_scoped_block(
                        cur,
                        stack,
                        sp,
                        span,
                        slice,
                        tok,
                        &mut lex,
                        allow_locals,
                        observer,
                    )?,
                TokenKind::Ident
                | TokenKind::PunctGe
                | TokenKind::PunctLe
                | TokenKind::PunctEqEq
                | TokenKind::PunctNe => {
                    let mut qbuf = [0u8; 64];
                    let (name, name_span) = if tok.kind == TokenKind::Ident {
                        let (len, used, s) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                        let bytes = if used {
                            &qbuf[..len]
                        } else {
                            &slice[tok.span.start..tok.span.end]
                        };
                        (bytes, s)
                    } else {
                        (&slice[tok.span.start..tok.span.end], tok.span)
                    };
                    let name_abs =
                        Span::new(span.start + name_span.start, span.start + name_span.end);
                    self.compile_name(
                        cur,
                        stack,
                        sp,
                        span,
                        slice,
                        &mut lex,
                        tok,
                        name,
                        name_abs,
                        allow_locals,
                        &mut terminated,
                        observer,
                    )?
                }
                _ => cur,
            };
            observer.on_token(stack, *sp, &slice[tok.span.start..tok.span.end]);
        }

        Ok(cur)
    }

}
