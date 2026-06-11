use super::*;

mod literals;
mod memory;
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
        allow_suspend: bool,
        allow_locals: bool,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        if quot_span.end <= quot_span.start + 2 {
            return Ok(cur);
        }
        let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
        self.compile_span(cur, stack, sp, inner, allow_suspend, allow_locals, observer)
    }

    pub(super) fn compile_span(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        allow_suspend: bool,
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
                TokenKind::PunctArrow => {
                    self.compile_field_access(cur, stack, sp, span, slice, tok, &mut lex)?
                }
                TokenKind::PunctApostrophe => self.compile_index(
                    cur,
                    stack,
                    sp,
                    span,
                    slice,
                    tok,
                    &mut lex,
                    allow_suspend,
                    allow_locals,
                    observer,
                )?,
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
                        allow_suspend,
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
                        allow_suspend,
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
