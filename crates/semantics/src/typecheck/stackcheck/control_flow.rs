use super::quote::typecheck_quote_body;
use super::*;
use ir::{Context, EffectSet};

#[allow(clippy::too_many_arguments)]
pub(super) fn do_if(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    nominals: &NominalDb,
    ctx: Context,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let else_q = pop(stack, sp).ok_or(TcError::IfPopElse {
        span: Span::UNKNOWN,
    })?;
    let then_q = pop(stack, sp).ok_or(TcError::IfPopThen {
        span: Span::UNKNOWN,
    })?;
    let cond = pop(stack, sp).ok_or(TcError::IfPopCond {
        span: Span::UNKNOWN,
    })?;
    if cond != Value::Plain(TypeAtom::BOOL) {
        return Err(TcError::IfCondNotBool {
            span: Span::UNKNOWN,
        });
    }
    let then_span = match then_q {
        Value::Quot(s) => s,
        _ => {
            return Err(TcError::IfThenNotQuot {
                span: Span::UNKNOWN,
            })
        }
    };
    let else_span = match else_q {
        Value::Quot(s) => s,
        _ => {
            return Err(TcError::IfElseNotQuot {
                span: Span::UNKNOWN,
            })
        }
    };

    let base_sp = *sp;
    let mut then_stack = *stack;
    let mut then_sp = base_sp;
    typecheck_quote_body(
        &mut then_stack,
        &mut then_sp,
        src,
        then_span,
        env,
        subtypes,
        mmio,
        nominals,
        ctx,
        out,
    )?;

    let mut else_stack = *stack;
    let mut else_sp = base_sp;
    typecheck_quote_body(
        &mut else_stack,
        &mut else_sp,
        src,
        else_span,
        env,
        subtypes,
        mmio,
        nominals,
        ctx,
        out,
    )?;

    if then_sp != else_sp {
        return Err(TcError::IfBranchDepth {
            span: Span::UNKNOWN,
        });
    }
    for i in 0..then_sp {
        if then_stack[i] != else_stack[i] {
            return Err(TcError::IfBranchContent {
                span: Span::UNKNOWN,
            });
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
    ctx: Context,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError::WhilePopBody {
        span: Span::UNKNOWN,
    })?;
    let cond_q = pop(stack, sp).ok_or(TcError::WhilePopCond {
        span: Span::UNKNOWN,
    })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => {
            return Err(TcError::WhileBodyNotQuot {
                span: Span::UNKNOWN,
            })
        }
    };
    let cond_span = match cond_q {
        Value::Quot(s) => s,
        _ => {
            return Err(TcError::WhileCondNotQuot {
                span: Span::UNKNOWN,
            })
        }
    };

    let base_sp = *sp;
    let base_stack = *stack;

    let mut cond_stack = base_stack;
    let mut cond_sp = base_sp;
    typecheck_quote_body(
        &mut cond_stack,
        &mut cond_sp,
        src,
        cond_span,
        env,
        subtypes,
        mmio,
        nominals,
        ctx,
        out,
    )?;
    if cond_sp != base_sp + 1 {
        return Err(TcError::WhileCondDepth {
            span: Span::UNKNOWN,
        });
    }
    if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::BOOL) {
        return Err(TcError::WhileCondNotBool {
            span: Span::UNKNOWN,
        });
    }
    // must preserve original stack below bool
    for i in 0..base_sp {
        if cond_stack[i] != base_stack[i] {
            return Err(TcError::WhileCondModifiedStack {
                span: Span::UNKNOWN,
            });
        }
    }

    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(
        &mut body_stack,
        &mut body_sp,
        src,
        body_span,
        env,
        subtypes,
        mmio,
        nominals,
        ctx,
        out,
    )?;
    if body_sp != base_sp {
        return Err(TcError::WhileBodyDepth {
            span: Span::UNKNOWN,
        });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError::WhileBodyModifiedStack {
                span: Span::UNKNOWN,
            });
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
    ctx: Context,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError::LoopPopBody {
        span: Span::UNKNOWN,
    })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => {
            return Err(TcError::LoopBodyNotQuot {
                span: Span::UNKNOWN,
            })
        }
    };

    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(
        &mut body_stack,
        &mut body_sp,
        src,
        body_span,
        env,
        subtypes,
        mmio,
        nominals,
        ctx,
        out,
    )?;
    if body_sp != base_sp {
        return Err(TcError::LoopBodyDepth {
            span: Span::UNKNOWN,
        });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError::LoopBodyModifiedStack {
                span: Span::UNKNOWN,
            });
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
    ctx: Context,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError::LockPopBody {
        span: Span::UNKNOWN,
    })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => {
            return Err(TcError::LockBodyNotQuot {
                span: Span::UNKNOWN,
            })
        }
    };
    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    let lock_ctx = Context::new(
        ctx.grants,
        ctx.forbids.union(EffectSet::from_bits(EffectSet::SUSPEND)),
        ctx.ceiling,
    );
    typecheck_quote_body(
        &mut body_stack,
        &mut body_sp,
        src,
        body_span,
        env,
        subtypes,
        mmio,
        nominals,
        lock_ctx,
        out,
    )?;
    if body_sp != base_sp {
        return Err(TcError::LockBodyDepth {
            span: Span::UNKNOWN,
        });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError::LockBodyModifiedStack {
                span: Span::UNKNOWN,
            });
        }
    }
    Ok(())
}
