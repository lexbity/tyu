use crate::typecheck::db::{struct_field_ty, NominalDb, SubtypeInfo};
use crate::typecheck::error::{ChecksMode, Output, TcError};
use crate::typecheck::mmio::{
    access_can_read, access_can_write, field_mask_shift, resolve_mmio_place, MmioDb, MmioResolved,
};
use crate::typecheck::parse::{
    capture_balanced, capture_scoped_block, parse_place, parse_type_expr, parse_word_sig,
    read_qualified_name,
};
use crate::typecheck::util::{
    apply_sig, array_elem_type, check_no_scoped_live, find_local, find_subtype, lookup, pop, push,
    slice_span, type_compatible, write_sig, write_stack, write_u64_dec, write_u64_hex,
};
use crate::typecheck::value::Value;
use crate::types::{TypeAtom, WordEntry, WordSig};
use frontend::lex::Lexer;
use frontend::parse::{DeclKind, ModuleAst};
use frontend::span::Span;
use frontend::token::TokenKind;
use ir::{CapSet, Context, EffectSet, High};

mod control_flow;
mod quote;
use self::control_flow::{do_if, do_lock, do_loop, do_while};

pub fn emit_stackcheck(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = crate::typecheck::mmio::build_mmio_db(module, src)?;
    let nominals = crate::typecheck::db::build_nominal_db(module, src)?;
    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            continue;
        };
        let Some(body_span) = decl.body else {
            continue;
        };
        let sig = parse_word_sig(src, sig_span)
            .map_err(|_| TcError::TypeParseFailed { span: sig_span })?;
        out.write(b"word ");
        out.write(slice_span(src, decl.name));
        out.write(b" ");
        write_sig(out, &sig);
        out.write(b"\n");
        let ctx = if decl.effect_bits & 1 != 0 {
            // Word declared with !{suspend} — body may suspend
            Context::default()
        } else {
            // Word NOT declared with !{suspend} — body must not suspend
            Context::new(
                CapSet::empty(),
                EffectSet::from_bits(EffectSet::SUSPEND),
                High::Top,
            )
        };
        typecheck_word_body(
            out, src, body_span, &sig, env, subtypes, &mmio, &nominals, checks, ctx,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn typecheck_word_body(
    out: &mut impl Output,
    src: &[u8],
    body_span: Span,
    declared: &WordSig,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    nominals: &NominalDb,
    checks: ChecksMode,
    ctx: Context,
) -> Result<(), TcError> {
    let slice = &src[body_span.start..body_span.end];
    let mut lex = Lexer::new(slice);

    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::EMPTY); 256];
    let mut sp: usize = 0;

    // Seed stack with declared inputs.
    for i in 0..(declared.in_len as usize) {
        stack[sp] = Value::Plain(declared.inputs[i]);
        sp += 1;
    }

    let mut locals: [TypeAtom; 64] = [TypeAtom::EMPTY; 64];
    let mut local_tys: [TypeAtom; 64] = [TypeAtom::EMPTY; 64];
    let mut local_len: usize = 0;

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
                push(&mut stack, &mut sp, Value::Plain(TypeAtom::I64))?;
                out.write(b"  ");
                out.write(&slice[tok.span.start..tok.span.end]);
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::String => {
                push(&mut stack, &mut sp, Value::Plain(TypeAtom::STR))?;
                out.write(b"  <str> | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctArrowBind => {
                let name = lex.next();
                if name.kind != TokenKind::Ident {
                    return Err(TcError::ExpectedIdent {
                        span: Span::new(
                            body_span.start + name.span.start,
                            body_span.start + name.span.end,
                        ),
                    });
                }
                let v = pop(&stack, &mut sp).ok_or(TcError::StackUnderflow {
                    span: Span::new(
                        body_span.start + tok.span.start,
                        body_span.start + tok.span.end,
                    ),
                })?;
                if v == Value::Plain(TypeAtom::SCOPED) {
                    return Err(TcError::BorrowEscape { span: body_span });
                }
                let ty = v.to_type_atom();
                let lname = TypeAtom::new(&slice[name.span.start..name.span.end]).ok_or(
                    TcError::TypeParseFailed {
                        span: Span::new(
                            body_span.start + name.span.start,
                            body_span.start + name.span.end,
                        ),
                    },
                )?;
                if find_local(&locals, local_len, lname).is_some() {
                    return Err(TcError::BindingAlreadyDefined {
                        span: Span::new(
                            body_span.start + name.span.start,
                            body_span.start + name.span.end,
                        ),
                    });
                }
                if local_len >= locals.len() {
                    return Err(TcError::BindingCapacityExceeded { span: body_span });
                }
                locals[local_len] = lname;
                local_tys[local_len] = ty;
                local_len += 1;
                out.write(b"  => ");
                out.write(lname.as_bytes());
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctArrow => {
                let op_span = Span::new(
                    body_span.start + tok.span.start,
                    body_span.start + tok.span.end,
                );
                let field_tok = lex.next();
                if field_tok.kind != TokenKind::Ident {
                    return Err(TcError::FieldNotFound { span: op_span });
                }
                let field_atom = TypeAtom::new(&slice[field_tok.span.start..field_tok.span.end])
                    .ok_or(TcError::FieldNotFound { span: op_span })?;
                if sp == 0 {
                    return Err(TcError::StackUnderflow { span: op_span });
                }
                let base = stack[sp - 1];
                let (struct_ty, mutable) = match base {
                    Value::Ptr { ty, mutable } => (ty, mutable),
                    _ => return Err(TcError::FieldNotFound { span: op_span }),
                };
                let field_ty = struct_field_ty(nominals, struct_ty, field_atom)
                    .ok_or(TcError::FieldNotFound { span: op_span })?;
                stack[sp - 1] = Value::Ptr {
                    ty: field_ty,
                    mutable,
                };
                out.write(b"  ->");
                out.write(field_atom.as_bytes());
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctLBracket => {
                // Capture the whole quotation span, including nested brackets.
                let q = capture_balanced(
                    &mut lex,
                    slice,
                    TokenKind::PunctLBracket,
                    TokenKind::PunctRBracket,
                    tok.span.start,
                )
                .map_err(|_| TcError::TypeParseFailed {
                    span: Span::new(
                        body_span.start + tok.span.start,
                        body_span.start + tok.span.end,
                    ),
                })?;
                let q_span = Span::new(body_span.start + q.start, body_span.start + q.end);
                push(&mut stack, &mut sp, Value::Quot(q_span))?;
                out.write(b"  <quot> | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctApostrophe => {
                let op_span = Span::new(
                    body_span.start + tok.span.start,
                    body_span.start + tok.span.end,
                );
                let next = lex.next();
                let mut has_dynamic = false;
                if next.kind == TokenKind::Number {
                    let _ = next;
                } else if next.kind == TokenKind::PunctLParen {
                    let _ = capture_balanced(
                        &mut lex,
                        slice,
                        TokenKind::PunctLParen,
                        TokenKind::PunctRParen,
                        next.span.start,
                    )
                    .map_err(|_| TcError::IndexError { span: op_span })?;
                    has_dynamic = true;
                } else {
                    return Err(TcError::IndexError { span: op_span });
                }

                if has_dynamic {
                    push(&mut stack, &mut sp, Value::Plain(TypeAtom::I64))?;
                }
                let base_pos = if has_dynamic {
                    sp.saturating_sub(2)
                } else {
                    sp.saturating_sub(1)
                };
                if base_pos >= sp {
                    return Err(TcError::IndexError { span: op_span });
                }
                let base = stack[base_pos];
                let result = match base {
                    Value::Plain(t) => {
                        let elem =
                            array_elem_type(t).ok_or(TcError::IndexError { span: op_span })?;
                        Value::Plain(elem)
                    }
                    Value::Ptr { mutable, .. } => {
                        let ty = if mutable {
                            TypeAtom::PTR_MUT
                        } else {
                            TypeAtom::PTR
                        };
                        Value::Plain(ty)
                    }
                    Value::MmioPlace(res) => Value::MmioPlace(res),
                    Value::MmioPtr { mutable, .. } => {
                        let ty = if mutable {
                            TypeAtom::PTR_MUT
                        } else {
                            TypeAtom::PTR
                        };
                        Value::Plain(ty)
                    }
                    _ => return Err(TcError::IndexError { span: op_span }),
                };

                // Drop index and base, push result.
                stack[base_pos] = result;
                sp = base_pos + 1;
                out.write(b"  idx | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                let mut_tok = tok.kind == TokenKind::PunctAmpBang;
                let place = parse_place(&mut lex, slice).ok_or(TcError::PlaceParseFailed {
                    span: Span::new(
                        body_span.start + tok.span.start,
                        body_span.start + tok.span.end,
                    ),
                })?;
                let place_bytes = &slice[place.full.start..place.full.end];
                let place_abs = Span::new(
                    body_span.start + place.full.start,
                    body_span.start + place.full.end,
                );

                if let Some(res) = resolve_mmio_place(mmio, src, place_bytes, place_abs)? {
                    match res {
                        MmioResolved::Reg(reg) => {
                            if mut_tok && !access_can_write(reg.access) {
                                return Err(TcError::MmioAccessViolation { span: place_abs });
                            }
                            push(
                                &mut stack,
                                &mut sp,
                                Value::MmioPtr {
                                    reg,
                                    mutable: mut_tok,
                                },
                            )?;
                        }
                        MmioResolved::Field(_) => {
                            // Fields are not addressable.
                            return Err(TcError::MmioFieldNotAddressable { span: place_abs });
                        }
                    }
                } else {
                    if mut_tok {
                        // locals are immutable in v1
                        let root = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap();
                        if find_local(&locals, local_len, root).is_some() {
                            return Err(TcError::MutRefToLocal {
                                span: place.root_abs(body_span.start),
                            });
                        }
                    }
                    let ty = if mut_tok {
                        TypeAtom::PTR_MUT
                    } else {
                        TypeAtom::PTR
                    };
                    push(&mut stack, &mut sp, Value::Plain(ty))?;
                }
                out.write(b"  ");
                out.write(if mut_tok { b"&!" } else { b"&" });
                out.write(place_bytes);
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
                if sp == 0 {
                    return Err(TcError::EmptyStackForScoped { span: body_span });
                }
                let top_ty = stack[sp - 1].to_type_atom();
                if array_elem_type(top_ty).is_none() && top_ty != TypeAtom::new(b"Region").unwrap()
                {
                    return Err(TcError::ScopedTypeMismatch { span: body_span });
                }

                // Non-IR checker uses the legacy marker to ensure "must be consumed by end of block".
                let scoped = Value::Plain(TypeAtom::SCOPED);
                push(&mut stack, &mut sp, scoped)?;

                let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|_| {
                    TcError::TypeParseFailed {
                        span: Span::new(
                            body_span.start + tok.span.start,
                            body_span.start + tok.span.end,
                        ),
                    }
                })?;
                let block_ctx = if mut_scope {
                    // &![ always forbids suspend
                    Context::new(
                        ctx.grants,
                        ctx.forbids.union(EffectSet::from_bits(EffectSet::SUSPEND)),
                        ctx.ceiling,
                    )
                } else {
                    ctx
                };
                typecheck_word_body(
                    out,
                    src,
                    Span::new(
                        body_span.start + block.inner_start,
                        body_span.start + block.inner_end,
                    ),
                    &WordSig::empty(), // Stack effects inside blocks not verified in simple stackcheck
                    env,
                    subtypes,
                    mmio,
                    nominals,
                    checks,
                    block_ctx,
                )?;

                if !check_no_scoped_live(&stack, sp) {
                    return Err(TcError::ScopedMarkerLeak { span: body_span });
                }
                out.write(b"  ");
                out.write(if mut_scope { b"&![...]" } else { b"&[...]" });
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::Ident
            | TokenKind::PunctGe
            | TokenKind::PunctLe
            | TokenKind::PunctEqEq
            | TokenKind::PunctNe => {
                let mut qbuf = [0u8; 64];
                let (name, name_span) = if tok.kind == TokenKind::Ident {
                    let (len, used, span) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                    let bytes = if used {
                        &qbuf[..len]
                    } else {
                        &slice[tok.span.start..tok.span.end]
                    };
                    (bytes, span)
                } else {
                    (&slice[tok.span.start..tok.span.end], tok.span)
                };
                if name == b"true" || name == b"false" {
                    push(&mut stack, &mut sp, Value::Plain(TypeAtom::BOOL))?;
                    out.write(b"  ");
                    out.write(name);
                    out.write(b" | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                let name_abs = Span::new(
                    body_span.start + name_span.start,
                    body_span.start + name_span.end,
                );
                if tok.kind == TokenKind::Ident
                    && !name.is_empty()
                    && (name[0] == b'@' || name[0] == b'!')
                {
                    // Typed loads/stores: `@u32` / `!u32` and untyped `@` / `!` for MMIO places.
                    let is_load = name[0] == b'@';
                    let typed = name.len() > 1;
                    let ty_atom = if typed {
                        Some(
                            TypeAtom::new(&name[1..])
                                .ok_or(TcError::MmioTypedAtomInvalid { span: name_abs })?,
                        ) // Corrected: Added missing '?'
                    } else {
                        None
                    };

                    if is_load {
                        let addr = pop(&stack, &mut sp)
                            .ok_or(TcError::MmioTypedPopAddr { span: name_abs })?;
                        match addr {
                            Value::MmioPtr { reg, .. } => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError::MmioReadNotAllowed { span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch {
                                            span: name_abs,
                                        });
                                    }
                                }
                                push(&mut stack, &mut sp, Value::Plain(reg.reg_ty))?;
                                out.write(b"  ");
                                out.write(if reg.volatile { b"vol_load " } else { b"load " });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError::MmioReadNotAllowed { span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch {
                                            span: name_abs,
                                        });
                                    }
                                }
                                push(&mut stack, &mut sp, Value::Plain(reg.reg_ty))?;
                                out.write(b"  ");
                                out.write(if reg.volatile { b"vol_load " } else { b"load " });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Field(field)) => {
                                if !access_can_read(field.reg_access)
                                    || !access_can_read(field.field.access)
                                {
                                    return Err(TcError::MmioReadNotAllowed { span: name_abs });
                                }
                                if typed {
                                    return Err(TcError::MmioTypedMismatch { span: name_abs });
                                }
                                push(&mut stack, &mut sp, Value::Plain(field.field.ty))?;
                                let (mask, shift) = field_mask_shift(&field.field);
                                out.write(b"  ");
                                out.write(if field.volatile {
                                    b"vol_load_field "
                                } else {
                                    b"load_field "
                                });
                                out.write(field.field.ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, field.place_span));
                                out.write(b" mask=0x");
                                write_u64_hex(out, mask);
                                out.write(b" shift=");
                                write_u64_dec(out, shift as u64);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            Value::Plain(t) => {
                                if !typed {
                                    return Err(TcError::MmioTypedNotAllowed { span: name_abs });
                                }
                                if t != TypeAtom::PTR && t != TypeAtom::PTR_MUT {
                                    return Err(TcError::MmioTypedNotAllowed { span: name_abs });
                                }
                                let want = ty_atom.expect("typed => ty_atom is Some");
                                push(&mut stack, &mut sp, Value::Plain(want))?;
                                out.write(b"  ");
                                out.write(name);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            _ => return Err(TcError::MmioTypedNotAllowed { span: name_abs }),
                        }
                    } else {
                        let val = pop(&stack, &mut sp)
                            .ok_or(TcError::MmioTypedPopAddr { span: name_abs })?;
                        let addr = pop(&stack, &mut sp)
                            .ok_or(TcError::MmioTypedPopAddr { span: name_abs })?;
                        match (addr, val) {
                            (Value::MmioPtr { reg, mutable }, Value::Plain(vty)) => {
                                if !mutable {
                                    return Err(TcError::MmioAccessViolation { span: name_abs });
                                }
                                if !access_can_write(reg.access) {
                                    return Err(TcError::MmioAccessViolation { span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch {
                                            span: name_abs,
                                        });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError::ReturnTypeMismatch { span: name_abs });
                                }
                                out.write(b"  ");
                                out.write(if reg.volatile {
                                    b"vol_store "
                                } else {
                                    b"store "
                                });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(vty)) => {
                                if !access_can_write(reg.access) {
                                    return Err(TcError::MmioAccessViolation { span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError::MmioTypedTypeMismatch {
                                            span: name_abs,
                                        });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError::ReturnTypeMismatch { span: name_abs });
                                }
                                out.write(b"  ");
                                out.write(if reg.volatile {
                                    b"vol_store "
                                } else {
                                    b"store "
                                });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(vty)) => {
                                if !access_can_write(field.reg_access)
                                    || !access_can_write(field.field.access)
                                {
                                    return Err(TcError::MmioAccessViolation { span: name_abs });
                                }
                                if typed {
                                    return Err(TcError::MmioTypedMismatch { span: name_abs });
                                }
                                if vty != field.field.ty {
                                    return Err(TcError::ReturnTypeMismatch { span: name_abs });
                                }
                                let (mask, shift) = field_mask_shift(&field.field);
                                out.write(b"  ");
                                out.write(if field.volatile {
                                    b"vol_store_field "
                                } else {
                                    b"store_field "
                                });
                                out.write(field.field.ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, field.place_span));
                                out.write(b" mask=0x");
                                write_u64_hex(out, mask);
                                out.write(b" shift=");
                                write_u64_dec(out, shift as u64);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            (Value::Plain(t), Value::Plain(_vty)) => {
                                if !typed {
                                    return Err(TcError::MmioTypedNotAllowed { span: name_abs });
                                }
                                if t != TypeAtom::PTR_MUT {
                                    return Err(TcError::MmioTypedNotAllowed { span: name_abs });
                                }
                                out.write(b"  ");
                                out.write(name);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            _ => return Err(TcError::MmioTypedNotAllowed { span: name_abs }),
                        }
                    }
                }

                if tok.kind == TokenKind::Ident {
                    if let Some(res) = resolve_mmio_place(mmio, src, name, name_abs)? {
                        push(&mut stack, &mut sp, Value::MmioPlace(res))?;
                        out.write(b"  mmio ");
                        out.write(name);
                        out.write(b" | stack: ");
                        write_stack(out, &stack, sp);
                        out.write(b"\n");
                        continue;
                    }
                }

                if name == b"dup" {
                    let top =
                        pop(&stack, &mut sp).ok_or(TcError::StackUnderflow { span: body_span })?;
                    push(&mut stack, &mut sp, top)?;
                    push(&mut stack, &mut sp, top)?;
                    out.write(b"  dup | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"drop" {
                    let _ =
                        pop(&stack, &mut sp).ok_or(TcError::StackUnderflow { span: body_span })?;
                    out.write(b"  drop | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"swap" {
                    let b =
                        pop(&stack, &mut sp).ok_or(TcError::StackUnderflow { span: body_span })?;
                    let a =
                        pop(&stack, &mut sp).ok_or(TcError::StackUnderflow { span: body_span })?;
                    push(&mut stack, &mut sp, b)?;
                    push(&mut stack, &mut sp, a)?;
                    out.write(b"  swap | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                if name == b"as" || name == b"as?" || name == b"bitcast" {
                    let first = lex.next();
                    let start = first.span.start;
                    let (ty_atom, next) =
                        parse_type_expr(slice, start).ok_or(TcError::CastParseFailed {
                            span: Span::new(
                                body_span.start + first.span.start,
                                body_span.start + first.span.end,
                            ),
                        })?;
                    lex.set_pos(next);
                    if name == b"as?" {
                        // ( base -- subtype ok ) for subtypes; for MVP treat others as identity + ok
                        let v = pop(&stack, &mut sp)
                            .ok_or(TcError::CastPopValue { span: body_span })?;
                        let got = v.to_type_atom();
                        if let Some(st) = find_subtype(subtypes, ty_atom) {
                            if !type_compatible(got, st.base, subtypes) {
                                return Err(TcError::CastSubtypeMismatch { span: body_span });
                            }
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                            push(&mut stack, &mut sp, Value::Plain(TypeAtom::BOOL))?;
                        } else {
                            // general: assume it can convert, return ok
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                            push(&mut stack, &mut sp, Value::Plain(TypeAtom::BOOL))?;
                        }
                        out.write(b"  ");
                        out.write(name);
                        out.write(b" ");
                        out.write(ty_atom.as_bytes());
                        out.write(b" | stack: ");
                        write_stack(out, &stack, sp);
                        out.write(b"\n");
                        continue;
                    }
                    if name == b"as" {
                        let v = pop(&stack, &mut sp)
                            .ok_or(TcError::CastPopValue { span: body_span })?;
                        let got = v.to_type_atom();
                        if let Some(st) = find_subtype(subtypes, ty_atom) {
                            if !type_compatible(got, st.base, subtypes) {
                                return Err(TcError::CastSubtypeMismatch { span: body_span });
                            }
                            if checks == ChecksMode::All {
                                out.write(b"  check_subtype ");
                                out.write(ty_atom.as_bytes());
                                out.write(b"\n");
                                out.write(b"  trap_if_false SUBTYPE_FAIL\n");
                            }
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                        } else {
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                        }
                        out.write(b"  as ");
                        out.write(ty_atom.as_bytes());
                        out.write(b" | stack: ");
                        write_stack(out, &stack, sp);
                        out.write(b"\n");
                        continue;
                    }
                }

                if name == b"if" {
                    do_if(
                        &mut stack, &mut sp, src, env, subtypes, mmio, nominals, ctx, out,
                    )?;
                    out.write(b"  if | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"while" {
                    do_while(
                        &mut stack, &mut sp, src, env, subtypes, mmio, nominals, ctx, out,
                    )?;
                    out.write(b"  while | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"loop" {
                    do_loop(
                        &mut stack, &mut sp, src, env, subtypes, mmio, nominals, ctx, out,
                    )?;
                    out.write(b"  loop | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"return" {
                    let want = declared.out_len as usize;
                    if sp != want {
                        return Err(TcError::ReturnStackDepth { span: body_span });
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
                        if !type_compatible(got, declared.outputs[i], subtypes) {
                            return Err(TcError::ReturnTypeMismatch { span: body_span });
                        }
                    }
                    terminated = true;
                    out.write(b"  return | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"lock" {
                    let mut probe = lex;
                    let next = probe.next();
                    if next.kind == TokenKind::PunctLBracket {
                        lex = probe;
                        let _block =
                            capture_scoped_block(&mut lex, slice, next.span).map_err(|_| {
                                TcError::TypeParseFailed {
                                    span: Span::new(
                                        body_span.start + next.span.start,
                                        body_span.start + next.span.end,
                                    ),
                                }
                            })?;
                        let full_span = Span::new(
                            body_span.start + next.span.start,
                            body_span.start + lex.pos(),
                        );
                        push(&mut stack, &mut sp, Value::Quot(full_span))?;
                    }
                    do_lock(
                        &mut stack, &mut sp, src, env, subtypes, mmio, nominals, ctx, out,
                    )?;
                    out.write(b"  lock | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                if let Some(idx) = find_local(
                    &locals,
                    local_len,
                    TypeAtom::new(name).unwrap_or(TypeAtom::EMPTY),
                ) {
                    push(&mut stack, &mut sp, Value::Plain(local_tys[idx]))?;
                    out.write(b"  ");
                    out.write(name);
                    out.write(b" | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                let entry = lookup(env, name).ok_or(TcError::WordNotFound {
                    span: Span::new(
                        body_span.start + name_span.start,
                        body_span.start + name_span.end,
                    ),
                })?;
                if !entry.performs.intersect(ctx.forbids).is_empty() {
                    return Err(TcError::SuspendForbidden {
                        span: Span::new(
                            body_span.start + name_span.start,
                            body_span.start + name_span.end,
                        ),
                    });
                }
                apply_sig(
                    &mut stack,
                    &mut sp,
                    entry,
                    Span::new(
                        body_span.start + name_span.start,
                        body_span.start + name_span.end,
                    ),
                    subtypes,
                )?;

                out.write(b"  ");
                out.write(name);
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            _ => {
                // ignore other punctuation in MVP
            }
        }
    }

    // At end, stack must match declared outputs.
    if !check_no_scoped_live(&stack, sp) {
        return Err(TcError::BorrowEscape { span: body_span });
    }
    if sp != declared.out_len as usize {
        return Err(TcError::OutputCountMismatch { span: body_span });
    }
    for (i, v) in stack.iter().enumerate().take(declared.out_len as usize) {
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
        if !type_compatible(got, declared.outputs[i], subtypes) {
            return Err(TcError::OutputTypeMismatch { span: body_span });
        }
    }

    Ok(())
}
