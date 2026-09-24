use super::*;

/// Compute the incremental bound from `prev` to `curr` in a monoid where
/// `prev.compose(x) == curr`.  Solves for `x` given the compose formula:
///
///   net(x) = net(curr) − net(prev)
///   high(x) = high(curr) shifted so that prev.net does not affect it
///            = max(0, high(curr) − net(prev))
///
/// In practice the only meaningful use is when `prev` was snapshotted before
/// a branch and `curr` is `self.acc` after the branch body compiled.
fn bound_delta(prev: StackBound, curr: StackBound) -> StackBound {
    let net = curr.net.wrapping_sub(prev.net);
    let high = match curr.high {
        High::Top => High::Top,
        High::Slots(h) => {
            let raw = (h as i32).wrapping_sub(prev.net as i32);
            if raw < 0 {
                High::Slots(0)
            } else {
                High::Slots(raw as u32)
            }
        }
    };
    StackBound { net, high }
}

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn compile_if(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let else_q = pop(stack, sp).ok_or(TcError::IfPopElse { span })?;
        let then_q = pop(stack, sp).ok_or(TcError::IfPopThen { span })?;
        let cond = pop(stack, sp).ok_or(TcError::IfPopCond { span })?;
        if cond != Value::Plain(TypeAtom::BOOL) {
            return Err(TcError::IfCondNotBool { span });
        }
        // `if` consumes its boolean condition at runtime — account the pop so
        // branch nets reflect the real stack delta (BUG-012).
        self.acc = self.acc.compose(StackBound {
            net: -1,
            high: High::Slots(0),
        });
        let then_span = match then_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::IfThenNotQuot { span }),
        };
        let else_span = match else_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::IfElseNotQuot { span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;
        let pre_if_acc = self.acc;

        let then_blk = self.new_block(&base_stack, base_sp, span)?;
        let else_blk = self.new_block(&base_stack, base_sp, span)?;
        // P5: seed the branch entry states from the pre-BrIf abstract state
        // (the branch-entry stack excludes the condition the BrIf consumes —
        // `base_sp` truncation).
        self.interp.seed_from(then_blk, cur, base_sp);
        self.interp.seed_from(else_blk, cur, base_sp);
        self.emit_op(
            cur,
            lir::OpKind::BrIf {
                then_tgt: then_blk,
                else_tgt: else_blk,
            },
            span,
        )?;

        let mut then_stack = base_stack;
        let mut then_sp = base_sp;
        self.acc = pre_if_acc;
        let then_end = self.compile_quote_span(
            then_blk,
            &mut then_stack,
            &mut then_sp,
            then_span,
            false,
            observer,
        )?;
        let then_bound = bound_delta(pre_if_acc, self.acc);

        let mut else_stack = base_stack;
        let mut else_sp = base_sp;
        self.acc = pre_if_acc;
        let else_end = self.compile_quote_span(
            else_blk,
            &mut else_stack,
            &mut else_sp,
            else_span,
            false,
            observer,
        )?;
        let else_bound = bound_delta(pre_if_acc, self.acc);

        if then_sp != else_sp {
            return Err(TcError::IfBranchDepth { span });
        }
        for i in 0..then_sp {
            if then_stack[i] != else_stack[i] {
                return Err(TcError::IfBranchContent { span });
            }
        }

        self.acc = pre_if_acc.compose(then_bound.branch_max(else_bound));

        let join_blk = self.new_block(&then_stack, then_sp, span)?;
        // P5: the join's abstract entry state = hull of the branch-end states
        // (§7.2 `BrIf` join row — precise where both ends are known).
        self.interp.seed_join(join_blk, then_end, else_end, then_sp);
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
        span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError::WhilePopBody { span })?;
        let cond_q = pop(stack, sp).ok_or(TcError::WhilePopCond { span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::WhileBodyNotQuot { span }),
        };
        let cond_span = match cond_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::WhileCondNotQuot { span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;
        let pre_while_acc = self.acc;

        let header = self.new_block(&base_stack, base_sp, span)?;
        // P5: seed the loop header from the pre-loop abstract state (the Br
        // is a no-op transfer).
        self.interp.seed_from(header, cur, base_sp);
        self.emit_op(cur, lir::OpKind::Br { target: header }, span)?;

        let mut cond_stack = base_stack;
        let mut cond_sp = base_sp;
        self.acc = pre_while_acc;
        let cond_end = self.compile_quote_span(
            header,
            &mut cond_stack,
            &mut cond_sp,
            cond_span,
            false,
            observer,
        )?;
        if cond_sp != base_sp + 1 {
            return Err(TcError::WhileCondDepth { span });
        }
        if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::BOOL) {
            return Err(TcError::WhileCondNotBool { span });
        }
        for i in 0..base_sp {
            if cond_stack[i] != base_stack[i] {
                return Err(TcError::WhileCondModifiedStack { span });
            }
        }
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(
            cond_end,
            lir::OpKind::BrIf {
                then_tgt: body_blk,
                else_tgt: after_blk,
            },
            span,
        )?;
        // P5: seed the body entry from the cond-false... the BrIf routed the
        // (net-zero) cond state to both successors; the body/exit abstract
        // states = the header state after the BrIf popped the condition.
        self.interp.seed_from(body_blk, cond_end, base_sp);
        self.interp.seed_from(after_blk, cond_end, base_sp);

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        self.acc = pre_while_acc;
        let body_end = self.compile_quote_span(
            body_blk,
            &mut body_stack,
            &mut body_sp,
            body_span,
            false,
            observer,
        )?;
        let body_bound = bound_delta(pre_while_acc, self.acc);
        if body_sp != base_sp {
            return Err(TcError::WhileBodyDepth { span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError::WhileBodyModifiedStack { span });
            }
        }
        // Loop body must be net-zero (stack-bound §3.2). bound = (0, high(body))
        let loop_bound = StackBound {
            net: 0,
            high: body_bound.high,
        };
        self.acc = pre_while_acc.compose(loop_bound);
        // S8: every loop construct is a potential DIVERGE source.
        self.word.performs = self
            .word
            .performs
            .union(EffectSet::from_bits(EffectSet::DIVERGE));
        // S9: loop in a bounded context.
        if self.ctx.ambient_forbids.contains(EffectSet::DIVERGE) {
            return Err(TcError::DivergeInBounded { span });
        }
        self.emit_op(body_end, lir::OpKind::Br { target: header }, span)?;
        // P5: back-edge widening (FR-12: loop-carried slots widen to ⊤) and
        // the loop-EXIT state reseeded from the (widened) header — a site
        // after the loop sees the widened values.
        self.interp.widen_loop(header, body_end, base_sp);
        self.interp.seed_from(after_blk, header, base_sp);

        *stack = base_stack;
        *sp = base_sp;
        Ok(after_blk)
    }

    pub(super) fn compile_loop(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError::LoopPopBody { span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::LoopBodyNotQuot { span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;
        let pre_loop_acc = self.acc;

        let check_blk = self.new_block(&base_stack, base_sp, span)?;
        // P5: seed the loop-check entry from the pre-loop abstract state.
        self.interp.seed_from(check_blk, cur, base_sp);
        self.emit_op(cur, lir::OpKind::Br { target: check_blk }, span)?;
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;

        self.emit_op(check_blk, lir::OpKind::ConstBool(true), span)?;
        self.emit_op(
            check_blk,
            lir::OpKind::BrIf {
                then_tgt: body_blk,
                else_tgt: after_blk,
            },
            span,
        )?;
        // P5: seed body/exit abstract states from the check state post-BrIf.
        self.interp.seed_from(body_blk, check_blk, base_sp);
        self.interp.seed_from(after_blk, check_blk, base_sp);

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        self.acc = pre_loop_acc;
        let body_end = self.compile_quote_span(
            body_blk,
            &mut body_stack,
            &mut body_sp,
            body_span,
            false,
            observer,
        )?;
        let body_bound = bound_delta(pre_loop_acc, self.acc);
        if body_sp != base_sp {
            return Err(TcError::LoopBodyDepth { span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError::LoopBodyModifiedStack { span });
            }
        }
        // Loop body must be net-zero (stack-bound §3.2). bound = (0, high(body))
        let loop_bound = StackBound {
            net: 0,
            high: body_bound.high,
        };
        self.acc = pre_loop_acc.compose(loop_bound);
        // S8: every loop construct is a potential DIVERGE source.
        self.word.performs = self
            .word
            .performs
            .union(EffectSet::from_bits(EffectSet::DIVERGE));
        // S9: 5040 — loop in a bounded context.
        if self.ctx.ambient_forbids.contains(EffectSet::DIVERGE) {
            return Err(TcError::DivergeInBounded { span });
        }
        self.emit_op(body_end, lir::OpKind::Br { target: check_blk }, span)?;
        // P5: back-edge widening + loop-exit reseed (as in `while`).
        self.interp.widen_loop(check_blk, body_end, base_sp);
        self.interp.seed_from(after_blk, check_blk, base_sp);

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
        // S4: lock rules flow exclusively through context stack frames.
        if self.ctx.lock_frame().is_some() {
            return Err(TcError::LockNest { span });
        }
        let body_q = pop(stack, sp).ok_or(TcError::LockPopBody { span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::LockBodyNotQuot { span }),
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
        let param = locked.map_or(FrameParam::None, FrameParam::Resource);
        self.ctx.push(ContextKind::Lock, param, span)?;
        let base_stack = *stack;
        let base_sp = *sp;
        let emit_irq_mask = locked
            .map(|name| resource_is_isr_reachable(self.resources, name))
            .unwrap_or(false);
        if emit_irq_mask {
            self.emit_op(cur, lir::OpKind::InterruptDisable, span)?;
        }
        let end = self.compile_quote_span(cur, stack, sp, body_span, true, observer)?;
        if emit_irq_mask {
            self.emit_op(end, lir::OpKind::InterruptEnable, span)?;
        }
        if *sp != base_sp {
            return Err(TcError::LockStack { span });
        }
        for i in 0..base_sp {
            if stack[i] != base_stack[i] {
                return Err(TcError::LockStack { span });
            }
        }
        self.ctx.pop();
        Ok(end)
    }
}
