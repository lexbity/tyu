use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {

    /// Check if a number token contains `e`/`E` (reserved float exponent syntax).
    /// Returns an error if found, unless the token is `0x`-prefixed (hex).
    fn check_float_exponent(bytes: &[u8], span: Span) -> Result<(), TcError> {
        if bytes.len() > 2 && bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X') {
            return Ok(()); // hex prefix — `e` is a valid hex digit
        }
        if bytes.iter().any(|&b| b == b'e' || b == b'E') {
            return Err(TcError::FloatExponent { span });
        }
        Ok(())
    }

    pub(super) fn compile_number(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        slice: &[u8],
        tok: Token,
    ) -> Result<lir::BlockId, TcError> {
        let tok_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
        let bytes = &slice[tok.span.start..tok.span.end];
        // D-15: reject `e`/`E` in non-hex number tokens (reserved float exponent).
        Self::check_float_exponent(bytes, tok_span)?;
        push(stack, sp, Value::Plain(TypeAtom::I64))?;
        let num = parse_i64_token(bytes).unwrap_or(0);
        self.emit_op(cur, lir::OpKind::ConstI64(num), tok_span)?;
        Ok(cur)
    }


    pub(super) fn compile_string(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        _slice: &[u8],
        tok: Token,
    ) -> Result<lir::BlockId, TcError> {
        push(stack, sp, Value::Plain(TypeAtom::STR))?;
        self.emit_op(
            cur,
            lir::OpKind::ConstStr(Span::new(
                span.start + tok.span.start,
                span.start + tok.span.end,
            )),
            Span::new(span.start + tok.span.start, span.start + tok.span.end),
        )?;
        Ok(cur)
    }


    pub(super) fn compile_bool_literal(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name: &[u8],
        name_abs: Span,
    ) -> Result<(), TcError> {
        push(stack, sp, Value::Plain(TypeAtom::BOOL))?;
        self.emit_op(cur, lir::OpKind::ConstBool(name == b"true"), name_abs)?;
        Ok(())
    }


    pub(super) fn compile_enum_variant(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        name_abs: Span,
        enum_ty: TypeAtom,
        var: TypeAtom,
    ) -> Result<bool, TcError> {
        if let Some(v) = enum_variant_value(self.nominals, enum_ty, var) {
            self.emit_op(cur, lir::OpKind::ConstI64(v), name_abs)?;
            push(stack, sp, Value::Plain(TypeAtom::I64))?;
            let to = self.ty_id_of_type(enum_ty, name_abs)?;
            self.emit_op(
                cur,
                lir::OpKind::Cast {
                    from: lir::TY_I64,
                    to,
                },
                name_abs,
            )?;
            let _ = pop(stack, sp);
            push(stack, sp, Value::Plain(enum_ty))?;
            return Ok(true);
        }
        for e in self.nominals.enums.iter() {
            if e.name == enum_ty {
                return Err(TcError::EnumVariantNotFound { span: name_abs });
            }
        }
        Ok(false)
    }


}
