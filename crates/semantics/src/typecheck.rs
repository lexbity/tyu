use crate::types::{SigParseError, TypeAtom, WordEntry, WordSig};
use frontend::{lex::Lexer, parse::DeclKind, parse::ModuleAst, span::Span, token::TokenKind};

pub trait Output {
    fn write(&mut self, bytes: &[u8]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcError {
    pub code: u32,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Value {
    Plain(TypeAtom),
    Quot(Span),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksMode {
    Off,
    Contracts,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubtypeInfo {
    pub name: TypeAtom,
    pub base: TypeAtom,
    pub min: i64,
    pub max: i64,
}

pub fn parse_word_sig(src: &[u8], sig_span: Span) -> Result<WordSig, SigParseError> {
    let slice = &src[sig_span.start..sig_span.end];
    let mut lex = Lexer::new(slice);
    let mut sig = WordSig::empty();
    let mut in_phase = true;

    loop {
        let t = lex.next();
        match t.kind {
            TokenKind::Eof => break,
            TokenKind::PunctLParen | TokenKind::PunctRParen => continue,
            TokenKind::PunctDashDash => {
                in_phase = false;
            }
            TokenKind::Ident => {
                let b = &slice[t.span.start..t.span.end];
                let atom = TypeAtom::new(b).ok_or(SigParseError {
                    code: 3101,
                    span: Span::new(sig_span.start + t.span.start, sig_span.start + t.span.end),
                })?;
                if in_phase {
                    let idx = sig.in_len as usize;
                    if idx >= sig.inputs.len() {
                        return Err(SigParseError {
                            code: 3102,
                            span: sig_span,
                        });
                    }
                    sig.inputs[idx] = atom;
                    sig.in_len += 1;
                } else {
                    let idx = sig.out_len as usize;
                    if idx >= sig.outputs.len() {
                        return Err(SigParseError {
                            code: 3103,
                            span: sig_span,
                        });
                    }
                    sig.outputs[idx] = atom;
                    sig.out_len += 1;
                }
            }
            _ => {
                return Err(SigParseError {
                    code: 3100,
                    span: Span::new(sig_span.start + t.span.start, sig_span.start + t.span.end),
                });
            }
        }
    }

    Ok(sig)
}

pub fn emit_ir(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    out: &mut impl Output,
) -> Result<(), TcError> {
    out.write(b"module ");
    out.write(slice_span(src, module.name));
    out.write(b"\n");

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError {
                code: 3200,
                span: decl.name,
            });
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError {
            code: e.code,
            span: e.span,
        })?;
        let name_bytes = slice_span(src, decl.name);
        out.write(b"word ");
        out.write(name_bytes);
        out.write(b" ");
        write_sig(out, &sig);
        out.write(b"\n");

        if checks == ChecksMode::All {
            for i in 0..(sig.in_len as usize) {
                if is_subtype(subtypes, sig.inputs[i]) {
                    out.write(b"  check_param ");
                    out.write(sig.inputs[i].as_bytes());
                    out.write(b"\n");
                    out.write(b"  trap_if_false SUBTYPE_FAIL\n");
                }
            }
        }

        if checks != ChecksMode::Off {
            if checks == ChecksMode::Contracts || checks == ChecksMode::All {
                if let Some(req) = decl.requires {
                    out.write(b"  requires ");
                    out.write(b"[...]");
                    out.write(b"\n");
                    check_contract_predicate(src, req, &sig, env, subtypes, out)?;
                }
            }
        }

        let Some(body_span) = decl.body else {
            continue;
        };
        typecheck_word_body(out, src, body_span, &sig, env, subtypes, checks)?;

        if checks != ChecksMode::Off {
            if checks == ChecksMode::Contracts || checks == ChecksMode::All {
                if let Some(ens) = decl.ensures {
                    out.write(b"  ensures ");
                    out.write(b"[...]");
                    out.write(b"\n");
                    check_ensures_predicate(src, ens, &sig, env, subtypes, out)?;
                }
            }
        }

        if checks == ChecksMode::All {
            for i in 0..(sig.out_len as usize) {
                if is_subtype(subtypes, sig.outputs[i]) {
                    out.write(b"  check_ret ");
                    out.write(sig.outputs[i].as_bytes());
                    out.write(b"\n");
                    out.write(b"  trap_if_false SUBTYPE_FAIL\n");
                }
            }
        }
    }

    Ok(())
}

fn typecheck_word_body(
    out: &mut impl Output,
    src: &[u8],
    body_span: Span,
    declared: &WordSig,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
) -> Result<(), TcError> {
    let slice = &src[body_span.start..body_span.end];
    let mut lex = Lexer::new(slice);

    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;

    // Seed stack with declared inputs.
    for i in 0..(declared.in_len as usize) {
        stack[sp] = Value::Plain(declared.inputs[i]);
        sp += 1;
    }

    let mut locals: [TypeAtom; 64] = [TypeAtom::new(b"").unwrap(); 64];
    let mut local_tys: [TypeAtom; 64] = [TypeAtom::new(b"").unwrap(); 64];
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
                push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?;
                out.write(b"  ");
                out.write(&slice[tok.span.start..tok.span.end]);
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::String => {
                push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"str").unwrap()))?;
                out.write(b"  <str> | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctArrowBind => {
                let name = lex.next();
                if name.kind != TokenKind::Ident {
                    return Err(TcError {
                        code: 3201,
                        span: Span::new(body_span.start + name.span.start, body_span.start + name.span.end),
                    });
                }
                let v = pop(&stack, &mut sp).ok_or(TcError {
                    code: 3202,
                    span: Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end),
                })?;
                let ty = match v {
                    Value::Plain(t) => t,
                    Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                };
                let lname = TypeAtom::new(&slice[name.span.start..name.span.end]).ok_or(TcError {
                    code: 3203,
                    span: Span::new(body_span.start + name.span.start, body_span.start + name.span.end),
                })?;
                if find_local(&locals, local_len, lname).is_some() {
                    return Err(TcError {
                        code: 3204,
                        span: Span::new(body_span.start + name.span.start, body_span.start + name.span.end),
                    });
                }
                if local_len >= locals.len() {
                    return Err(TcError { code: 3205, span: body_span });
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
            TokenKind::PunctLBracket => {
                // Capture the whole quotation span, including nested brackets.
                let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                    .map_err(|code| TcError { code, span: Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end) })?;
                let q_span = Span::new(body_span.start + q.start, body_span.start + q.end);
                push(&mut stack, &mut sp, Value::Quot(q_span))?;
                out.write(b"  <quot> | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::Ident
            | TokenKind::PunctGe
            | TokenKind::PunctLe
            | TokenKind::PunctEqEq
            | TokenKind::PunctNe => {
                let name = &slice[tok.span.start..tok.span.end];
                if name == b"true" || name == b"false" {
                    push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                    out.write(b"  ");
                    out.write(name);
                    out.write(b" | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                if name == b"dup" {
                    let top = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    push(&mut stack, &mut sp, top)?;
                    push(&mut stack, &mut sp, top)?;
                    out.write(b"  dup | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"drop" {
                    let _ = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    out.write(b"  drop | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"swap" {
                    let b = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    let a = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    push(&mut stack, &mut sp, b)?;
                    push(&mut stack, &mut sp, a)?;
                    out.write(b"  swap | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                if name == b"as" || name == b"as?" || name == b"bitcast" {
                    let ty = lex.next();
                    if ty.kind != TokenKind::Ident {
                        return Err(TcError {
                            code: 3295,
                            span: Span::new(body_span.start + ty.span.start, body_span.start + ty.span.end),
                        });
                    }
                    let ty_atom = TypeAtom::new(&slice[ty.span.start..ty.span.end]).ok_or(TcError {
                        code: 3296,
                        span: Span::new(body_span.start + ty.span.start, body_span.start + ty.span.end),
                    })?;
                    if name == b"as?" {
                        // ( base -- subtype ok ) for subtypes; for MVP treat others as identity + ok
                        let v = pop(&stack, &mut sp).ok_or(TcError { code: 3297, span: body_span })?;
                        let got = match v {
                            Value::Plain(t) => t,
                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                        };
                        if let Some(st) = find_subtype(subtypes, ty_atom) {
                            if !type_compatible(got, st.base, subtypes) {
                                return Err(TcError { code: 3298, span: body_span });
                            }
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                            push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                        } else {
                            // general: assume it can convert, return ok
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                            push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
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
                        let v = pop(&stack, &mut sp).ok_or(TcError { code: 3299, span: body_span })?;
                        let got = match v {
                            Value::Plain(t) => t,
                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                        };
                        if let Some(st) = find_subtype(subtypes, ty_atom) {
                            if !type_compatible(got, st.base, subtypes) {
                                return Err(TcError { code: 3300, span: body_span });
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
                    do_if(&mut stack, &mut sp, src, env, subtypes)?;
                    out.write(b"  if | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"while" {
                    do_while(&mut stack, &mut sp, src, env, subtypes)?;
                    out.write(b"  while | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"loop" {
                    do_loop(&mut stack, &mut sp, src, env, subtypes)?;
                    out.write(b"  loop | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"return" {
                    let want = declared.out_len as usize;
                    if sp != want {
                        return Err(TcError { code: 3230, span: body_span });
                    }
                    for i in 0..want {
                        let got = match stack[i] {
                            Value::Plain(t) => t,
                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                        };
                        if !type_compatible(got, declared.outputs[i], subtypes) {
                            return Err(TcError { code: 3231, span: body_span });
                        }
                    }
                    terminated = true;
                    out.write(b"  return | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"lock" {
                    do_lock(&mut stack, &mut sp, src, env, subtypes)?;
                    out.write(b"  lock | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                if let Some(idx) = find_local(&locals, local_len, TypeAtom::new(name).unwrap_or(TypeAtom::new(b"").unwrap())) {
                    push(&mut stack, &mut sp, Value::Plain(local_tys[idx]))?;
                    out.write(b"  ");
                    out.write(name);
                    out.write(b" | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                let sig = lookup(env, name).ok_or(TcError {
                    code: 3210,
                    span: Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end),
                })?;
                apply_sig(
                    &mut stack,
                    &mut sp,
                    &sig,
                    Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end),
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
    if sp != declared.out_len as usize {
        return Err(TcError { code: 3220, span: body_span });
    }
    for i in 0..(declared.out_len as usize) {
        let got = match stack[i] {
            Value::Plain(t) => t,
            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
        };
        if !type_compatible(got, declared.outputs[i], subtypes) {
            return Err(TcError { code: 3221, span: body_span });
        }
    }

    Ok(())
}

fn do_if(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    let else_q = pop(stack, sp).ok_or(TcError { code: 3240, span: Span::new(0, 0) })?;
    let then_q = pop(stack, sp).ok_or(TcError { code: 3241, span: Span::new(0, 0) })?;
    let cond = pop(stack, sp).ok_or(TcError { code: 3242, span: Span::new(0, 0) })?;
    if cond != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3243, span: Span::new(0, 0) });
    }
    let then_span = match then_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3244, span: Span::new(0, 0) }),
    };
    let else_span = match else_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3245, span: Span::new(0, 0) }),
    };

    let base_sp = *sp;
    let mut then_stack = *stack;
    let mut then_sp = base_sp;
    typecheck_quote_body(&mut then_stack, &mut then_sp, src, then_span, env, subtypes)?;

    let mut else_stack = *stack;
    let mut else_sp = base_sp;
    typecheck_quote_body(&mut else_stack, &mut else_sp, src, else_span, env, subtypes)?;

    if then_sp != else_sp {
        return Err(TcError { code: 3246, span: Span::new(0, 0) });
    }
    for i in 0..then_sp {
        if then_stack[i] != else_stack[i] {
            return Err(TcError { code: 3247, span: Span::new(0, 0) });
        }
    }

    for i in 0..then_sp {
        stack[i] = then_stack[i];
    }
    *sp = then_sp;
    Ok(())
}

fn do_while(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3250, span: Span::new(0, 0) })?;
    let cond_q = pop(stack, sp).ok_or(TcError { code: 3251, span: Span::new(0, 0) })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3252, span: Span::new(0, 0) }),
    };
    let cond_span = match cond_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3253, span: Span::new(0, 0) }),
    };

    let base_sp = *sp;
    let base_stack = *stack;

    let mut cond_stack = base_stack;
    let mut cond_sp = base_sp;
    typecheck_quote_body(&mut cond_stack, &mut cond_sp, src, cond_span, env, subtypes)?;
    if cond_sp != base_sp + 1 {
        return Err(TcError { code: 3254, span: Span::new(0, 0) });
    }
    if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3255, span: Span::new(0, 0) });
    }
    // must preserve original stack below bool
    for i in 0..base_sp {
        if cond_stack[i] != base_stack[i] {
            return Err(TcError { code: 3256, span: Span::new(0, 0) });
        }
    }

    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3257, span: Span::new(0, 0) });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3258, span: Span::new(0, 0) });
        }
    }
    Ok(())
}

fn do_loop(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3260, span: Span::new(0, 0) })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3261, span: Span::new(0, 0) }),
    };

    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3262, span: Span::new(0, 0) });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3263, span: Span::new(0, 0) });
        }
    }
    Ok(())
}

fn do_lock(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3270, span: Span::new(0, 0) })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3271, span: Span::new(0, 0) }),
    };
    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3272, span: Span::new(0, 0) });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3273, span: Span::new(0, 0) });
        }
    }
    Ok(())
}

fn typecheck_quote_body(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    quot_span: Span,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    // Expect brackets at ends; just slice inside.
    if quot_span.end <= quot_span.start + 2 {
        return Ok(());
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
            TokenKind::Number => push(stack, sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?,
            TokenKind::Ident => {
                let name = &slice[tok.span.start..tok.span.end];
                if name == b"true" || name == b"false" {
                    push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                    continue;
                }
                if name == b"dup" {
                    let top = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    push(stack, sp, top)?;
                    push(stack, sp, top)?;
                    continue;
                }
                if name == b"drop" {
                    let _ = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    continue;
                }
                if name == b"swap" {
                    let b = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    let a = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    push(stack, sp, b)?;
                    push(stack, sp, a)?;
                    continue;
                }
                if name == b"if" {
                    do_if(stack, sp, src, env, subtypes)?;
                    continue;
                }
                if name == b"while" {
                    do_while(stack, sp, src, env, subtypes)?;
                    continue;
                }
                if name == b"loop" {
                    do_loop(stack, sp, src, env, subtypes)?;
                    continue;
                }
                if name == b"lock" {
                    do_lock(stack, sp, src, env, subtypes)?;
                    continue;
                }
                let sig = lookup(env, name).ok_or(TcError { code: 3280, span: quot_span })?;
                apply_sig(stack, sp, &sig, quot_span, subtypes)?;
            }
            TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                // treat as word-like operator
                let name = &slice[tok.span.start..tok.span.end];
                let sig = lookup(env, name).ok_or(TcError { code: 3280, span: quot_span })?;
                apply_sig(stack, sp, &sig, quot_span, subtypes)?;
            }
            TokenKind::PunctArrowBind => {
                // locals not allowed inside quotations in MVP; ignore
                return Err(TcError { code: 3281, span: quot_span });
            }
            TokenKind::PunctLBracket => {
                let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                    .map_err(|code| TcError { code, span: quot_span })?;
                let q_span = Span::new(inner.start + q.start, inner.start + q.end);
                push(stack, sp, Value::Quot(q_span))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn lookup<'a>(env: &'a [WordEntry], name: &[u8]) -> Option<WordSig> {
    for e in env {
        if e.name.as_bytes() == name {
            return Some(e.sig);
        }
    }
    None
}

fn apply_sig(
    stack: &mut [Value; 256],
    sp: &mut usize,
    sig: &WordSig,
    span: Span,
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    let need = sig.in_len as usize;
    if *sp < need {
        return Err(TcError { code: 3211, span });
    }
    // Check types from top.
    for i in 0..need {
        let got = stack[*sp - need + i];
        let got = match got {
            Value::Plain(t) => t,
            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
        };
        if !type_compatible(got, sig.inputs[i], subtypes) {
            return Err(TcError { code: 3212, span });
        }
    }
    *sp -= need;
    for i in 0..(sig.out_len as usize) {
        push(stack, sp, Value::Plain(sig.outputs[i]))?;
    }
    Ok(())
}

fn push(stack: &mut [Value; 256], sp: &mut usize, v: Value) -> Result<(), TcError> {
    if *sp >= stack.len() {
        return Err(TcError { code: 3206, span: Span::new(0, 0) });
    }
    stack[*sp] = v;
    *sp += 1;
    Ok(())
}

fn pop(stack: &[Value; 256], sp: &mut usize) -> Option<Value> {
    if *sp == 0 {
        return None;
    }
    *sp -= 1;
    Some(stack[*sp])
}

fn find_local(locals: &[TypeAtom; 64], len: usize, name: TypeAtom) -> Option<usize> {
    let mut i = 0usize;
    while i < len {
        if locals[i] == name {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn write_sig(out: &mut impl Output, sig: &WordSig) {
    out.write(b"( ");
    for i in 0..(sig.in_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        out.write(sig.inputs[i].as_bytes());
    }
    out.write(b" --");
    if sig.out_len > 0 {
        out.write(b" ");
    }
    for i in 0..(sig.out_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        out.write(sig.outputs[i].as_bytes());
    }
    out.write(b" )");
}

fn write_stack(out: &mut impl Output, stack: &[Value; 256], sp: usize) {
    for i in 0..sp {
        if i != 0 {
            out.write(b" ");
        }
        match stack[i] {
            Value::Plain(t) => out.write(t.as_bytes()),
            Value::Quot(_) => out.write(b"quot"),
        }
    }
}

fn capture_balanced(
    lex: &mut Lexer<'_>,
    slice: &[u8],
    open: TokenKind,
    close: TokenKind,
    open_start: usize,
) -> Result<Span, u32> {
    let mut depth = 1usize;
    let mut end = open_start + 1;
    while depth > 0 {
        let t = lex.next();
        if t.kind == TokenKind::Eof {
            return Err(3290);
        }
        end = t.span.end;
        if t.kind == open {
            depth += 1;
        } else if t.kind == close {
            depth -= 1;
        } else if t.kind == TokenKind::String {
            // already handled in lexer
        }
        let _ = slice;
    }
    Ok(Span::new(open_start, end))
}

fn slice_span<'a>(src: &'a [u8], span: Span) -> &'a [u8] {
    &src[span.start..span.end]
}

fn find_subtype(subtypes: &[SubtypeInfo], name: TypeAtom) -> Option<SubtypeInfo> {
    for &s in subtypes {
        if s.name == name {
            return Some(s);
        }
    }
    None
}

fn is_subtype(subtypes: &[SubtypeInfo], ty: TypeAtom) -> bool {
    find_subtype(subtypes, ty).is_some()
}

fn type_compatible(got: TypeAtom, want: TypeAtom, subtypes: &[SubtypeInfo]) -> bool {
    if got == want {
        return true;
    }
    // subtype <: base
    if let Some(st) = find_subtype(subtypes, got) {
        if st.base == want {
            return true;
        }
    }
    false
}

fn check_contract_predicate(
    src: &[u8],
    quot_span: Span,
    sig: &WordSig,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    out: &mut impl Output,
) -> Result<(), TcError> {
    // Seed stack with declared inputs, and require predicate ends with (inputs + bool).
    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;
    for i in 0..(sig.in_len as usize) {
        stack[sp] = Value::Plain(sig.inputs[i]);
        sp += 1;
    }
    let base_sp = sp;
    typecheck_quote_body(&mut stack, &mut sp, src, quot_span, env, subtypes)?;
    if sp != base_sp + 1 {
        return Err(TcError { code: 3310, span: quot_span });
    }
    if stack[sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3311, span: quot_span });
    }
    // Must preserve types below bool.
    for i in 0..base_sp {
        if stack[i] != Value::Plain(sig.inputs[i]) {
            return Err(TcError { code: 3312, span: quot_span });
        }
    }
    out.write(b"  trap_if_false CONTRACT_FAIL\n");
    Ok(())
}

fn check_ensures_predicate(
    src: &[u8],
    quot_span: Span,
    sig: &WordSig,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;
    for i in 0..(sig.out_len as usize) {
        stack[sp] = Value::Plain(sig.outputs[i]);
        sp += 1;
    }
    let base_sp = sp;
    typecheck_quote_body(&mut stack, &mut sp, src, quot_span, env, subtypes)?;
    if sp != base_sp + 1 {
        return Err(TcError { code: 3320, span: quot_span });
    }
    if stack[sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3321, span: quot_span });
    }
    for i in 0..base_sp {
        if stack[i] != Value::Plain(sig.outputs[i]) {
            return Err(TcError { code: 3322, span: quot_span });
        }
    }
    out.write(b"  trap_if_false CONTRACT_FAIL\n");
    Ok(())
}
