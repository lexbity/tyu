use super::*;
use crate::typecheck::context::{ContextKind, FrameParam};

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn compile_task_spawn(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name_abs: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError::TaskSpawnPop { span: name_abs })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::TaskSpawnPop { span: name_abs }),
        };
        // `platform.task.spawn` consumes the task-body quotation (BUG-012);
        // the TaskSpawn op itself pushes the task handle (+1).
        self.acc = self.acc.compose(StackBound {
            net: -1,
            high: High::Slots(0),
        });
        let (qname, qsig, _performs, _qbound) = self.build_quote_word(body_span, observer)?;
        if qsig.in_len != 0 || qsig.out_len != 0 {
            return Err(TcError::TaskSpawnSig { span: name_abs });
        }

        let task_ty = TypeAtom::new(b"Task").ok_or(TcError::TaskSpawnType { span: name_abs })?;
        let task_tid = self.ty_id_of_type(task_ty, name_abs)?;
        push(stack, sp, Value::Plain(task_ty))?;
        self.emit_op(
            cur,
            lir::OpKind::TaskSpawn {
                name: qname,
                task_ty: task_tid,
            },
            name_abs,
        )?;
        Ok(cur)
    }

    pub(super) fn compile_task_run(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name_abs: Span,
        _span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        // S7: suspend gate at the run call site — forbidders (Lock, MutBorrow,
        // Isr) and ReadBorrow liveness stop the call.  Undeclared is NOT
        // checked because Handler discharges SUSPEND for the body.
        if let Some(s) = self
            .ctx
            .forbidding_span(EffectSet::from_bits(EffectSet::SUSPEND))
        {
            return Err(TcError::SuspendForbidden { span: s });
        }
        if self.any_scoped_live(stack, *sp) {
            return Err(TcError::SuspendForbidden { span: name_abs });
        }
        let body_q = pop(stack, sp).ok_or(TcError::TaskRunPop { span: name_abs })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::TaskRunNotQuot { span: name_abs }),
        };
        // `platform.task.run` consumes the handler-body quotation (BUG-012).
        self.acc = self.acc.compose(StackBound {
            net: -1,
            high: High::Slots(0),
        });
        let base_stack = *stack;
        let base_sp = *sp;

        // S7: snapshot performs before compiling the body, so we can compute
        // the delta and discharge SUSPEND from outward propagation.
        let before = self.word.performs;
        self.ctx
            .push(ContextKind::Handler, FrameParam::None, name_abs)?;
        cur = self.compile_quote_span(cur, stack, sp, body_span, false, observer)?;
        self.ctx.pop();
        let body_delta = self.word.performs.minus(before);
        // Discharge SUSPEND: the body may have added SUSPEND (via yield),
        // but the handler strips it from what propagates outward.
        self.word.performs = before.union(body_delta.without(EffectSet::SUSPEND));

        if *sp != base_sp {
            return Err(TcError::TaskRunDepth { span: name_abs });
        }
        for i in 0..base_sp {
            if stack[i] != base_stack[i] {
                return Err(TcError::TaskRunModified { span: name_abs });
            }
        }
        Ok(cur)
    }
}
