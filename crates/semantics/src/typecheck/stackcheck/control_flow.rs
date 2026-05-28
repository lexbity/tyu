use super::*;
use super::quote::typecheck_quote_body;

#[allow(clippy::too_many_arguments)]
pub(super) fn do_if(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    nominals: &NominalDb,
    allow_suspend: bool,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let else_q = pop(stack, sp).ok_or(TcError { code: 3240, span: Span::UNKNOWN })?;
    let then_q = pop(stack, sp).ok_or(TcError { code: 3241, span: Span::UNKNOWN })?;
    let cond = pop(stack, sp).ok_or(TcError { code: 3242, span: Span::UNKNOWN })?;
    if cond != Value::Plain(TypeAtom::BOOL) {
        return Err(TcError { code: 3243, span: Span::UNKNOWN });
    }
    let then_span = match then_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3244, span: Span::UNKNOWN }),
    };
    let else_span = match else_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3245, span: Span::UNKNOWN }),
    };

    let base_sp = *sp;
    let mut then_stack = *stack;
    let mut then_sp = base_sp;
    typecheck_quote_body(&mut then_stack, &mut then_sp, src, then_span, env, subtypes, mmio, nominals, allow_suspend, out)?;

    let mut else_stack = *stack;
    let mut else_sp = base_sp;
    typecheck_quote_body(&mut else_stack, &mut else_sp, src, else_span, env, subtypes, mmio, nominals, allow_suspend, out)?;

    if then_sp != else_sp {
        return Err(TcError { code: 3246, span: Span::UNKNOWN });
    }
    for i in 0..then_sp {
        if then_stack[i] != else_stack[i] {
            return Err(TcError { code: 3247, span: Span::UNKNOWN });
        }
    }

    stack[..then_sp].copy_from_slice(&then_stack[..then_sp]);
    *sp = then_sp;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn do_while(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    nominals: &NominalDb,
    allow_suspend: bool,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3250, span: Span::UNKNOWN })?;
    let cond_q = pop(stack, sp).ok_or(TcError { code: 3251, span: Span::UNKNOWN })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3252, span: Span::UNKNOWN }),
    };
    let cond_span = match cond_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3253, span: Span::UNKNOWN }),
    };

    let base_sp = *sp;
    let base_stack = *stack;

    let mut cond_stack = base_stack;
    let mut cond_sp = base_sp;
    typecheck_quote_body(&mut cond_stack, &mut cond_sp, src, cond_span, env, subtypes, mmio, nominals, allow_suspend, out)?;
    if cond_sp != base_sp + 1 {
        return Err(TcError { code: 3254, span: Span::UNKNOWN });
    }
    if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::BOOL) {
        return Err(TcError { code: 3255, span: Span::UNKNOWN });
    }
    // must preserve original stack below bool
    for i in 0..base_sp {
        if cond_stack[i] != base_stack[i] {
            return Err(TcError { code: 3256, span: Span::UNKNOWN });
        }
    }

    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes, mmio, nominals, allow_suspend, out)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3257, span: Span::UNKNOWN });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3258, span: Span::UNKNOWN });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn do_loop(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    nominals: &NominalDb,
    allow_suspend: bool,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3260, span: Span::UNKNOWN })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3261, span: Span::UNKNOWN }),
    };

    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes, mmio, nominals, allow_suspend, out)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3262, span: Span::UNKNOWN });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3263, span: Span::UNKNOWN });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn do_lock(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    nominals: &NominalDb,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3270, span: Span::UNKNOWN })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3271, span: Span::UNKNOWN }),
    };
    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    // lock is non-suspending
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes, mmio, nominals, false, out)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3272, span: Span::UNKNOWN });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3273, span: Span::UNKNOWN });
        }
    }
    Ok(())
}
