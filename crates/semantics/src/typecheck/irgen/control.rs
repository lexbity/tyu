use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn compile_if(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let else_q = pop(stack, sp).ok_or(TcError { code: 3240, span })?;
        let then_q = pop(stack, sp).ok_or(TcError { code: 3241, span })?;
        let cond = pop(stack, sp).ok_or(TcError { code: 3242, span })?;
        if cond != Value::Plain(TypeAtom::BOOL) {
            return Err(TcError { code: 3243, span });
        }
        let then_span = match then_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3244, span }),
        };
        let else_span = match else_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3245, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let then_blk = self.new_block(&base_stack, base_sp, span)?;
        let else_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::BrIf { then_tgt: then_blk, else_tgt: else_blk }, span)?;

        let mut then_stack = base_stack;
        let mut then_sp = base_sp;
        let then_end = self.compile_quote_span(then_blk, &mut then_stack, &mut then_sp, then_span, allow_suspend, false, observer)?;

        let mut else_stack = base_stack;
        let mut else_sp = base_sp;
        let else_end = self.compile_quote_span(else_blk, &mut else_stack, &mut else_sp, else_span, allow_suspend, false, observer)?;

        if then_sp != else_sp {
            return Err(TcError { code: 3246, span });
        }
        for i in 0..then_sp {
            if then_stack[i] != else_stack[i] {
                return Err(TcError { code: 3247, span });
            }
        }

        let join_blk = self.new_block(&then_stack, then_sp, span)?;
        self.emit_op(then_end, lir::OpKind::Br { target: join_blk }, span)?;
        self.emit_op(else_end, lir::OpKind::Br { target: join_blk }, span)?;

    stack[..then_sp].copy_from_slice(&then_stack[..then_sp]);
    *sp = then_sp;
    Ok(join_blk)
    }

    pub(super) fn compile_while(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError { code: 3250, span })?;
        let cond_q = pop(stack, sp).ok_or(TcError { code: 3251, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3252, span }),
        };
        let cond_span = match cond_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3253, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let header = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::Br { target: header }, span)?;

        let mut cond_stack = base_stack;
        let mut cond_sp = base_sp;
        let cond_end = self.compile_quote_span(header, &mut cond_stack, &mut cond_sp, cond_span, allow_suspend, false, observer)?;
        if cond_sp != base_sp + 1 {
            return Err(TcError { code: 3254, span });
        }
        if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::BOOL) {
            return Err(TcError { code: 3255, span });
        }
        for i in 0..base_sp {
            if cond_stack[i] != base_stack[i] {
                return Err(TcError { code: 3256, span });
            }
        }
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cond_end, lir::OpKind::BrIf { then_tgt: body_blk, else_tgt: after_blk }, span)?;

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        let body_end = self.compile_quote_span(body_blk, &mut body_stack, &mut body_sp, body_span, allow_suspend, false, observer)?;
        if body_sp != base_sp {
            return Err(TcError { code: 3257, span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError { code: 3258, span });
            }
        }
        self.emit_op(body_end, lir::OpKind::Br { target: header }, span)?;

        *stack = base_stack;
        *sp = base_sp;
        Ok(after_blk)
    }

    pub(super) fn compile_loop(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError { code: 3260, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3261, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let check_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::Br { target: check_blk }, span)?;
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;

        self.emit_op(check_blk, lir::OpKind::ConstBool(true), span)?;
        self.emit_op(check_blk, lir::OpKind::BrIf { then_tgt: body_blk, else_tgt: after_blk }, span)?;

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        let body_end = self.compile_quote_span(body_blk, &mut body_stack, &mut body_sp, body_span, allow_suspend, false, observer)?;
        if body_sp != base_sp {
            return Err(TcError { code: 3262, span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError { code: 3263, span });
            }
        }
        self.emit_op(body_end, lir::OpKind::Br { target: check_blk }, span)?;

        *stack = base_stack;
        *sp = base_sp;
        Ok(after_blk)
    }

    pub(super) fn compile_lock(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        if self.locked_resource.is_some() {
            return Err(TcError { code: 3517, span });
        }
        let body_q = pop(stack, sp).ok_or(TcError { code: 3270, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3271, span }),
        };
        let locked = if *sp > 0 {
            match stack[*sp - 1] {
                Value::Resource(name) => {
                    let _ = pop(stack, sp);
                    Some(name)
                }
                _ => None,
            }
        } else {
            None
        };
        self.locked_resource = locked;
        let base_stack = *stack;
        let base_sp = *sp;
        let end = self.compile_quote_span(cur, stack, sp, body_span, false, true, observer)?;
        if *sp != base_sp {
            return Err(TcError { code: 3272, span });
        }
        for i in 0..base_sp {
            if stack[i] != base_stack[i] {
                return Err(TcError { code: 3273, span });
            }
        }
        self.locked_resource = None;
        Ok(end)
    }
}
