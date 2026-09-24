use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn emit_prologue(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        requires: Option<Span>,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let mut cur = cur;
        let n = self.sig.in_len as usize;
        for i in (0..n).rev() {
            let v = pop(stack, sp).ok_or(TcError::StackUnderflow {
                span: Span::new(0, 0),
            })?;
            let _ = v;
            self.emit_op(
                cur,
                lir::OpKind::LocalSet {
                    slot: i as u16,
                    ty: self.word.sig.inputs[i],
                },
                Span::new(0, 0),
            )?;
        }

        // C1: record one obligation per subtype-typed input, then emit the
        // runtime trap per the emission mode. The record is independent of
        // `--checks` (FR-1 — the artifact is complete even when the trap is
        // not inserted); it uses the SAME decision as the emission
        // (`find_subtype`); never re-derives it. Under `Undischarged` the
        // record's resolved verdict gates the emission (P4); under `All` the
        // gate is a constant true — the emitted machine code is identical to
        // today's path (FR-5).
        for i in 0..n {
            if let Some(st) = find_subtype(self.subtypes, self.sig.inputs[i]) {
                let verdict = self.record_subtype_param_obligation(i, &st);
                if self.emit_subtype_check(verdict) {
                    self.emit_subtype_range_trap(
                        cur,
                        i as u16,
                        self.word.sig.inputs[i],
                        &st,
                        Span::new(0, 0),
                    )?;
                }
            }
        }

        let mut params_on_stack = false;
        if self.checks != ChecksMode::Off
            && (self.checks == ChecksMode::Contracts
                || self.checks == ChecksMode::All
                || self.checks == ChecksMode::Undischarged)
        {
            if let Some(req) = requires {
                for i in 0..n {
                    self.emit_op(
                        cur,
                        lir::OpKind::LocalGet {
                            slot: i as u16,
                            ty: self.word.sig.inputs[i],
                        },
                        req,
                    )?;
                    push(stack, sp, Value::Plain(self.sig.inputs[i]))?;
                }
                cur = self.compile_quote_span(cur, stack, sp, req, false, observer)?;
                if *sp != n + 1 {
                    return Err(TcError::ContractDepth { span: req });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::BOOL) {
                    return Err(TcError::ContractNotBool { span: req });
                }
                for (i, v) in stack.iter().enumerate().take(n) {
                    if *v != Value::Plain(self.sig.inputs[i]) {
                        return Err(TcError::ContractModifiedInputs { span: req });
                    }
                }
                let _ = pop(stack, sp);
                self.emit_op(
                    cur,
                    lir::OpKind::TrapIfFalse {
                        code: lir::TrapCode::ContractFail,
                    },
                    req,
                )?;
                // P4: one emitted contract trap — the honesty count behind
                // the report's `emitted.contract` field (FR-15).
                if let Some(ctx) = self.extraction.as_mut() {
                    ctx.note_emitted_contract();
                }
                params_on_stack = true;
            }
        }

        if !params_on_stack {
            for i in 0..n {
                self.emit_op(
                    cur,
                    lir::OpKind::LocalGet {
                        slot: i as u16,
                        ty: self.word.sig.inputs[i],
                    },
                    Span::new(0, 0),
                )?;
                push(stack, sp, Value::Plain(self.sig.inputs[i]))?;
            }
        }
        Ok(cur)
    }

    pub(super) fn emit_epilogue(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        ensures: Option<Span>,
        observer: &mut dyn TypecheckObserver,
    ) -> Result<lir::BlockId, TcError> {
        let mut cur = cur;

        if self.checks != ChecksMode::Off
            && (self.checks == ChecksMode::Contracts
                || self.checks == ChecksMode::All
                || self.checks == ChecksMode::Undischarged)
        {
            if let Some(ens) = ensures {
                let n = self.sig.out_len as usize;
                let base_sp = *sp;
                cur = self.compile_quote_span(cur, stack, sp, ens, false, observer)?;
                if *sp != base_sp + 1 {
                    return Err(TcError::ContractDepth { span: ens });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::BOOL) {
                    return Err(TcError::ContractNotBool { span: ens });
                }
                for (i, v) in stack.iter().enumerate().take(n) {
                    if *v != Value::Plain(self.sig.outputs[i]) {
                        return Err(TcError::ContractModifiedInputs { span: ens });
                    }
                }
                let _ = pop(stack, sp);
                self.emit_op(
                    cur,
                    lir::OpKind::TrapIfFalse {
                        code: lir::TrapCode::ContractFail,
                    },
                    ens,
                )?;
                // P4: one emitted contract trap (FR-15 honesty count).
                if let Some(ctx) = self.extraction.as_mut() {
                    ctx.note_emitted_contract();
                }
            }
        }

        // C2: record one obligation per subtype-typed output, then emit the
        // runtime trap per the emission mode. Under `All`/`Undischarged` the
        // values are staged through temp slots so the checks run before
        // return; the record is independent of the emission decision (FR-1).
        {
            let n = self.sig.out_len as usize;
            if self.checks == ChecksMode::All || self.checks == ChecksMode::Undischarged {
                let base_stack = *stack;
                let base_sp = *sp;
                let tmp_base = self.temp_base_slot();
                for i in (0..n).rev() {
                    let v = pop(stack, sp).ok_or(TcError::StackUnderflow {
                        span: Span::new(0, 0),
                    })?;
                    let _ = v;
                    self.emit_op(
                        cur,
                        lir::OpKind::LocalSet {
                            slot: tmp_base + i as u16,
                            ty: self.word.sig.outputs[i],
                        },
                        Span::new(0, 0),
                    )?;
                }
                for i in 0..n {
                    if let Some(st) = find_subtype(self.subtypes, self.sig.outputs[i]) {
                        let verdict = self.record_subtype_return_obligation(i, &st);
                        if self.emit_subtype_check(verdict) {
                            self.emit_subtype_range_trap(
                                cur,
                                tmp_base + i as u16,
                                self.word.sig.outputs[i],
                                &st,
                                Span::new(0, 0),
                            )?;
                        }
                    }
                }
                for i in 0..n {
                    self.emit_op(
                        cur,
                        lir::OpKind::LocalGet {
                            slot: tmp_base + i as u16,
                            ty: self.word.sig.outputs[i],
                        },
                        Span::new(0, 0),
                    )?;
                    stack[i] = base_stack[i];
                }
                *sp = base_sp;
            } else {
                // Off/Contracts: extraction only (FR-1).
                for i in 0..n {
                    if let Some(st) = find_subtype(self.subtypes, self.sig.outputs[i]) {
                        self.record_subtype_return_obligation(i, &st);
                    }
                }
            }
        }

        Ok(cur)
    }
}
