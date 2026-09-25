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
                let verdict = self.record_subtype_param_obligation(cur, i, &st);
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
                // Slice P6 (C5 site / Q6): record the callee-side `contract-pre`
                // obligation for this word's own `needs` clause. The verdict
                // gates the prologue trap under `Undischarged` (elided only at
                // a discharged site, FR-13); under `All`/`Contracts` every
                // contract site resolves open and emits (FR-5 — byte-identical).
                let c5_verdict = {
                    let word_name = self.word.name;
                    self.record_contract_pre_obligation(word_name.as_bytes(), n as u8, req)
                };
                // Q6: a predicate is a question — compile it under the
                // ContractPredicate context frame (the ambient fold forbids
                // every effect → E3313 on effect-performing calls) with the
                // store/spawn-free guards and the E3314 peak-size tracker
                // armed.
                let origin_snap = self.predicate_origin_snapshot(cur, n);
                self.begin_contract_predicate(req)?;
                cur = self.compile_quote_span(cur, stack, sp, req, false, observer)?;
                self.end_contract_predicate(req)?;
                if *sp != n + 1 {
                    return Err(TcError::ContractDepth { span: req });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::BOOL) {
                    return Err(TcError::ContractNotBool { span: req });
                }
                // E3312 (slice P6, FR-8): the input slots' ORIGINS must be
                // preserved — the check finally means "the predicate left the
                // subjects untouched", not merely "same static type". An
                // identity move (`swap`), a drop-and-replace (`drop true
                // true`), or any recomputation *of the subject slot* fires;
                // arithmetic on copies (`dup 1 + …`) passes.
                if !self.predicate_origins_preserved(cur, n, &origin_snap) {
                    return Err(TcError::ContractModifiedInputs { span: req });
                }
                let _ = pop(stack, sp);
                if self.emit_contract_check(c5_verdict) {
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
                } else if self.checks == ChecksMode::Undischarged {
                    // The site is discharged/assumed: the runtime trap is
                    // elided, but the predicate's evaluation still left its
                    // verdict on the stack — consume it so the IR stays
                    // balanced (the elision removes the *check*, never the
                    // predicate's shape). No honesty count: no trap emitted.
                    let tid = self.ty_id_of_type(TypeAtom::BOOL, req)?;
                    self.emit_op(cur, lir::OpKind::Drop { ty: tid }, req)?;
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

    /// The E3312 check (slice P6): the abstract slots of stack positions
    /// `0..n` must be unchanged since the predicate body began. Snapshot
    /// before the predicate compiles ([`Self::predicate_origin_snapshot`]),
    /// compare after. The comparison is over the interpreter's `Slot`
    /// `(Interval × Origin)` pair: the Origin lattice catches identity moves
    /// (`swap` — the E3312 hole), the Interval catches value replacements
    /// (`drop true true`, `1 +` on the subject). The subject slots of a
    /// *legal* predicate are only ever `dup`/joined — both stay fixed.
    fn predicate_origin_snapshot(&self, cur: lir::BlockId, n: usize) -> [verifier::interp::Slot; 8] {
        let st = self.interp.state_or_fresh(cur);
        let mut snap = [verifier::interp::Slot::top(); 8];
        for i in 0..n {
            snap[i] = st.stack.get(i).copied().unwrap_or(verifier::interp::Slot::top());
        }
        snap
    }

    fn predicate_origins_preserved(
        &self,
        cur: lir::BlockId,
        n: usize,
        snap: &[verifier::interp::Slot; 8],
    ) -> bool {
        let st = self.interp.state_or_fresh(cur);
        for i in 0..n {
            let now = st.stack.get(i).copied().unwrap_or(verifier::interp::Slot::top());
            if now != snap[i] {
                return false;
            }
        }
        true
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
                // Slice P6 (C6 site / Q6): record the `contract-post`
                // obligation BEFORE the predicate compiles, so the emitted
                // trap decision sits with the emit site. The abstract verdict
                // the inline predicate compilation leaves on the interval
                // state's top is the in-tree discharge input (captured after
                // the predicate, before the stack touches it again).
                let origin_snap = self.predicate_origin_snapshot(cur, n);
                self.begin_contract_predicate(ens)?;
                cur = self.compile_quote_span(cur, stack, sp, ens, false, observer)?;
                self.end_contract_predicate(ens)?;
                if *sp != base_sp + 1 {
                    return Err(TcError::ContractDepth { span: ens });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::BOOL) {
                    return Err(TcError::ContractNotBool { span: ens });
                }
                // E3312 (FR-8): the output slots' origins must be preserved
                // (mirror of the prologue check — an ensures predicate may not
                // move or replace the results it claims about).
                if !self.predicate_origins_preserved(cur, n, &origin_snap) {
                    return Err(TcError::ContractModifiedInputs { span: ens });
                }
                let pred_iv = self.interp.state_or_fresh(cur).top_interval();
                let c6_verdict = self.record_contract_post_obligation(pred_iv, ens);
                let _ = pop(stack, sp);
                if self.emit_contract_check(c6_verdict) {
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
                } else if self.checks == ChecksMode::Undischarged {
                    // Discharged site: elide the trap, consume the verdict.
                    let tid = self.ty_id_of_type(TypeAtom::BOOL, ens)?;
                    self.emit_op(cur, lir::OpKind::Drop { ty: tid }, ens)?;
                }
            }
        }

        // C2: record one obligation per subtype-typed output, then emit the
        // runtime trap per the emission mode. Under `All`/`Undischarged` the
        // values are staged through temp slots so the checks run before
        // return; the record is independent of the emission decision (FR-1).
        {
            let n = self.sig.out_len as usize;
            // P5: the outputs' abstract values, captured at epilogue entry
            // (before the staging moves them to temp slots) — the in-tree
            // discharge evaluates the return-value obligations from them.
            let out_ivs = self.interp.state_or_fresh(cur).top_n_intervals(n);
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
                        let verdict = self.record_subtype_return_obligation(i, &st, &out_ivs);
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
                        self.record_subtype_return_obligation(i, &st, &out_ivs);
                    }
                }
            }
        }

        Ok(cur)
    }
}
