use super::*;

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
        _allow_suspend: bool,
        _span: Span,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        if !self.check_no_scoped_live_all(stack, *sp) {
            return Err(TcError::ScopedLiveAtSuspend { span: name_abs });
        }
        let body_q = pop(stack, sp).ok_or(TcError::TaskRunPop { span: name_abs })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError::TaskRunNotQuot { span: name_abs }),
        };
        let base_stack = *stack;
        let base_sp = *sp;
        cur = self.compile_quote_span(cur, stack, sp, body_span, true, false, observer)?;
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
