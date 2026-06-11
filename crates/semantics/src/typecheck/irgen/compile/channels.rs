use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {

    pub(super) fn compile_channel_send(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        tok: Token,
    ) -> Result<lir::BlockId, TcError> {
        let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
        let val = pop(stack, sp).ok_or(TcError::ChanSendPop { span: op_span })?;
        let ch = pop(stack, sp).ok_or(TcError::ChanSendPop { span: op_span })?;

        let val_ty = val.to_type_atom();
        let ch_ty = match ch {
            Value::Plain(t) => t,
            _ => return Err(TcError::ChanSendType { span: op_span }),
        };
        let elem = chan_elem_type(ch_ty).ok_or(TcError::ChanSendType { span: op_span })?;
        if !type_compatible(val_ty, elem, self.subtypes) {
            return Err(TcError::ChanSendValueMismatch { span: op_span });
        }

        let ch_tid = self.ty_id_of_type(ch_ty, op_span)?;
        let val_tid = self.ty_id_of_type(val_ty, op_span)?;
        let mut sig = lir::Sig::empty();
        sig.in_len = 2;
        sig.out_len = 0;
        sig.inputs[0] = ch_tid;
        sig.inputs[1] = val_tid;
        self.emit_op(
            cur,
            lir::OpKind::Call {
                name: lir::Atom::new(b"platform.channel.send").unwrap(),
                sig,
                performs: EffectSet::empty(),
                requires: CapSet::empty(),
                bound: StackBound::ID,
            },
            op_span,
        )?;
        Ok(cur)
    }


    pub(super) fn compile_channel_recv(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        tok: Token,
    ) -> Result<lir::BlockId, TcError> {
        let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
        let ch = pop(stack, sp).ok_or(TcError::ChanRecvPop { span: op_span })?;
        let ch_ty = match ch {
            Value::Plain(t) => t,
            _ => return Err(TcError::ChanRecvType { span: op_span }),
        };
        let elem = chan_elem_type(ch_ty).ok_or(TcError::ChanRecvType { span: op_span })?;
        push(stack, sp, Value::Plain(elem))?;
        self.acc = self.acc.compose(StackBound {
            net: 0,
            high: High::Slots(1),
        });

        let ch_tid = self.ty_id_of_type(ch_ty, op_span)?;
        let elem_tid = self.ty_id_of_type(elem, op_span)?;
        let mut sig = lir::Sig::empty();
        sig.in_len = 1;
        sig.out_len = 1;
        sig.inputs[0] = ch_tid;
        sig.outputs[0] = elem_tid;
        self.emit_op(
            cur,
            lir::OpKind::Call {
                name: lir::Atom::new(b"platform.channel.recv").unwrap(),
                sig,
                performs: EffectSet::empty(),
                requires: CapSet::empty(),
                bound: StackBound::ID,
            },
            op_span,
        )?;
        Ok(cur)
    }


}
