use super::*;
use super::control_flow::{do_if, do_while, do_loop, do_lock};

#[allow(clippy::too_many_arguments)]
pub(super) fn typecheck_quote_body(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    quot_span: Span,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    nominals: &NominalDb,
    allow_suspend: bool,
    out: &mut impl Output,
) -> Result<(), TcError> {
    // Expect brackets at ends; just slice inside.
    if quot_span.end <= quot_span.start + 2 {
        return Ok(())
    }
    let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
    let slice = &src[inner.start..inner.end];
    let mut lex = Lexer::new(slice);

    // Optional leading signature + effect set for escaping quotations: ignore in v1 MVP.
    if lex.next().kind == TokenKind::PunctLParen {
        // rewind not possible: manually parse again with balanced skip
        lex = Lexer::new(slice);
        let first = lex.next();
        if first.kind == TokenKind::PunctLParen {
            let _ = capture_balanced(&mut lex, slice, TokenKind::PunctLParen, TokenKind::PunctRParen, first.span.start);
            let maybe_eff = lex.next();
            if maybe_eff.kind != TokenKind::EffectSet {
                // step back not supported; ok to proceed after consuming one token too far only if it's ws, but lexer skips ws.
                // So: only treat it as effect-set if it is.
                // If it isn't, we just continue with it as first term by re-lexing from its start.
                lex = Lexer::new(&slice[maybe_eff.span.start..]);
            }
        }
    } else {
        // first token consumed; re-lex from start
        lex = Lexer::new(slice);
    }

    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            break;
        }
        match tok.kind {
            TokenKind::Number => push(stack, sp, Value::Plain(TypeAtom::I64))?,
            TokenKind::Ident => {
                let mut qbuf = [0u8; 64];
                let (len, used, name_span) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                let name = if used {
                    &qbuf[..len]
                } else {
                    &slice[tok.span.start..tok.span.end]
                };
                if name == b"true" || name == b"false" {
                    push(stack, sp, Value::Plain(TypeAtom::BOOL))?;
                    continue;
                }
                let abs = Span::new(inner.start + name_span.start, inner.start + name_span.end);
                if !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
                    let is_load = name[0] == b'@';
                    let typed = name.len() > 1;
                    let ty_atom = if typed {
                        Some(TypeAtom::new(&name[1..]).ok_or(TcError::MmioTypedAtomInvalid { span: abs })?) // Corrected: Added missing '?'
                    } else {
                        None
                    };

                    if is_load {
                        let addr = pop(stack, sp).ok_or(TcError::MmioTypedPopAddr { span: abs })?;
                        match addr {
                            Value::MmioPtr { reg, .. } => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError::MmioReadNotAllowed { span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch { span: abs });
                                    }
                                }
                                push(stack, sp, Value::Plain(reg.reg_ty))?;
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError::MmioReadNotAllowed { span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch { span: abs });
                                    }
                                }
                                push(stack, sp, Value::Plain(reg.reg_ty))?;
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Field(field)) => {
                                if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
                                    return Err(TcError::MmioReadNotAllowed { span: abs });
                                }
                                if typed {
                                    return Err(TcError::MmioTypedMismatch { span: abs });
                                }
                                push(stack, sp, Value::Plain(field.field.ty))?;
                                continue;
                            }
                            Value::Plain(t) => {
                                if !typed {
                                    return Err(TcError::MmioTypedNotAllowed { span: abs });
                                }
                                if t != TypeAtom::PTR && t != TypeAtom::PTR_MUT {
                                    return Err(TcError::MmioTypedNotAllowed { span: abs });
                                }
                                push(stack, sp, Value::Plain(ty_atom.expect("typed => ty_atom is Some")))?;
                                continue;
                            }
                            _ => return Err(TcError::MmioTypedNotAllowed { span: abs }),
                        }
                    } else {
                        let val = pop(stack, sp).ok_or(TcError::MmioTypedPopAddr { span: abs })?;
                        let addr = pop(stack, sp).ok_or(TcError::MmioTypedPopAddr { span: abs })?;
                        match (addr, val) {
                            (Value::MmioPtr { reg, mutable }, Value::Plain(vty)) => {
                                if !mutable || !access_can_write(reg.access) {
                                    return Err(TcError::MmioAccessViolation { span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch { span: abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError::ReturnTypeMismatch { span: abs });
                                }
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(vty)) => {
                                if !access_can_write(reg.access) {
                                    return Err(TcError::MmioAccessViolation { span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch { span: abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError::ReturnTypeMismatch { span: abs });
                                }
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(vty)) => {
                                if !access_can_write(field.reg_access) || !access_can_write(field.field.access) {
                                    return Err(TcError::MmioAccessViolation { span: abs });
                                }
                                if typed {
                                    return Err(TcError::MmioTypedMismatch { span: abs });
                                }
                                if vty != field.field.ty {
                                    return Err(TcError::ReturnTypeMismatch { span: abs });
                                }
                                continue;
                            }
                            (Value::Plain(t), Value::Plain(_vty)) => {
                                if !typed {
                                    return Err(TcError::MmioTypedNotAllowed { span: abs });
                                }
                                if t != TypeAtom::PTR_MUT {
                                    return Err(TcError::MmioTypedNotAllowed { span: abs });
                                }
                                continue;
                            }
                            _ => return Err(TcError::MmioTypedNotAllowed { span: abs }),
                        }
                    }
                }

                if let Some(res) = resolve_mmio_place(mmio, src, name, abs)? {
                    push(stack, sp, Value::MmioPlace(res))?;
                    continue;
                }
                if name == b"dup" {
                    let top = pop(stack, sp).ok_or(TcError::StackUnderflow { span: quot_span })?;
                    push(stack, sp, top)?;
                    push(stack, sp, top)?;
                    continue;
                }
                if name == b"drop" {
                    let _ = pop(stack, sp).ok_or(TcError::StackUnderflow { span: quot_span })?;
                    continue;
                }
                if name == b"swap" {
                    let b = pop(stack, sp).ok_or(TcError::StackUnderflow { span: quot_span })?;
                    let a = pop(stack, sp).ok_or(TcError::StackUnderflow { span: quot_span })?;
                    push(stack, sp, b)?;
                    push(stack, sp, a)?;
                    continue;
                }
                if name == b"if" {
                    do_if(stack, sp, src, env, subtypes, mmio, nominals, allow_suspend, out)?;
                    continue;
                }
                if name == b"while" {
                    do_while(stack, sp, src, env, subtypes, mmio, nominals, allow_suspend, out)?;
                    continue;
                }
                if name == b"loop" {
                    do_loop(stack, sp, src, env, subtypes, mmio, nominals, allow_suspend, out)?;
                    continue;
                }
                if name == b"lock" {
                    let mut probe = lex;
                    let next = probe.next();
                    if next.kind == TokenKind::PunctLBracket {
                        lex = probe;
                        let _block = capture_scoped_block(&mut lex, slice, next.span)
                            .map_err(|_| TcError::TypeParseFailed { span: Span::new(quot_span.start + next.span.start, quot_span.start + next.span.end) })?;
                        let full_span = Span::new(quot_span.start + next.span.start, quot_span.start + lex.pos());
                        push(stack, sp, Value::Quot(full_span))?;
                    }
                    do_lock(stack, sp, src, env, subtypes, mmio, nominals, out)?;
                    continue;
                }
                let entry = lookup(env, name).ok_or(TcError::WordNotFound { span: quot_span })?;
                if entry.may_suspend && !allow_suspend {
                    return Err(TcError::SuspendingInNonSuspendingContext { span: quot_span });
                }
                apply_sig(stack, sp, entry, quot_span, subtypes)?;
            }
            TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                // treat as word-like operator
                let name = &slice[tok.span.start..tok.span.end];
                let entry = lookup(env, name).ok_or(TcError::WordNotFound { span: quot_span })?;
                if entry.may_suspend && !allow_suspend {
                    return Err(TcError::SuspendingInNonSuspendingContext { span: quot_span });
                }
                apply_sig(stack, sp, entry, quot_span, subtypes)?;
            }
            TokenKind::PunctArrowBind => {
                // locals not allowed inside quotations in MVP; ignore
                return Err(TcError::BindNotAllowed { span: quot_span });
            }
            TokenKind::PunctArrow => {
                let op_span = Span::new(quot_span.start + tok.span.start, quot_span.start + tok.span.end);
                let field_tok = lex.next();
                if field_tok.kind != TokenKind::Ident {
                    return Err(TcError::FieldNotFound { span: op_span });
                }
                let field_atom = TypeAtom::new(&slice[field_tok.span.start..field_tok.span.end])
                    .ok_or(TcError::FieldNotFound { span: op_span })?;
                if *sp == 0 {
                    return Err(TcError::StackUnderflow { span: op_span });
                }
                let base = stack[*sp - 1];
                let (struct_ty, mutable) = match base {
                    Value::Ptr { ty, mutable } => (ty, mutable),
                    _ => return Err(TcError::FieldNotFound { span: op_span }),
                };
                let field_ty = struct_field_ty(nominals, struct_ty, field_atom)
                    .ok_or(TcError::FieldNotFound { span: op_span })?;
                stack[*sp - 1] = Value::Ptr { ty: field_ty, mutable };
            }
            TokenKind::PunctLBracket => {
                let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                    .map_err(|_| TcError::TypeParseFailed { span: quot_span })?;
                let _ = q;
                push(stack, sp, Value::Quot(Span::UNKNOWN))?;
            }
            _ => {} // ignore other punctuation in MVP
        }
    }
    Ok(())
}
