use super::*;

impl<'a, 'r> IrWordGen<'a, 'r> {
    pub(super) fn enum_base_ty(&self, enum_ty: TypeAtom) -> Option<TypeAtom> {
        for e in self.nominals.enums.iter() {
            if e.name == enum_ty {
                return Some(e.base);
            }
        }
        None
    }

    pub(super) fn prim_bits_signed(&self, ty: TypeAtom) -> Option<(u16, bool)> {
        let b = ty.as_bytes();
        if b.starts_with(b"Chan(") {
            return Some((64, false));
        }
        let (bits, signed) = match b {
            b"u8" => (8, false),
            b"u16" => (16, false),
            b"u32" => (32, false),
            b"u64" => (64, false),
            b"usize" => (64, false),
            b"i8" => (8, true),
            b"i16" => (16, true),
            b"i32" => (32, true),
            b"i64" => (64, true),
            b"isize" => (64, true),
            b"bool" => (8, false),
            b"ptr" | b"ptr_mut" | b"str" | b"mmio" => (64, false),
            _ => return None,
        };
        Some((bits, signed))
    }

    pub(super) fn ty_bits_signed(&self, ty: TypeAtom) -> Option<(u16, bool)> {
        if let Some(p) = self.prim_bits_signed(ty) {
            return Some(p);
        }
        if let Some(base) = self.enum_base_ty(ty) {
            return self.prim_bits_signed(base);
        }
        if let Some(st) = find_subtype(self.subtypes, ty) {
            return self.prim_bits_signed(st.base);
        }
        None
    }

    pub(super) fn check_raw_cast_allowed(&self, from_ty: TypeAtom, to_ty: TypeAtom) -> bool {
        let from = from_ty.as_bytes();
        let to = to_ty.as_bytes();
        let is_ptrish = |t: &[u8]| matches!(t, b"ptr" | b"ptr_mut");
        let is_intish = |t: &[u8]| matches!(
            t,
            b"u8" | b"u16" | b"u32" | b"u64" | b"usize" | b"i8" | b"i16" | b"i32" | b"i64" | b"isize"
        );

        if is_ptrish(from) && is_intish(to) {
            return self.allow_raw_casts;
        }
        if is_intish(from) && is_ptrish(to) {
            return self.allow_raw_casts;
        }
        if is_ptrish(from) && is_ptrish(to) && from != to {
            return self.allow_raw_casts;
        }
        true
    }

    pub(super) fn ty_id_of_type(&mut self, ty: TypeAtom, span: Span) -> Result<lir::TypeId, TcError> {
        intern_type(&mut self.word.types, &mut self.word.type_sizes, lir_atom(ty.as_bytes())?, self.nominals, span)
    }

    pub(super) fn ty_id_of_value(&mut self, v: Value, span: Span) -> Result<lir::TypeId, TcError> {
        match v {
            Value::Plain(t) => self.ty_id_of_type(t, span),
            Value::Scoped { ty, .. } => self.ty_id_of_type(ty, span),
            Value::Resource(_) => intern_type(&mut self.word.types, &mut self.word.type_sizes, lir::AT_RESOURCE, self.nominals, span),
            Value::Quot(_) => intern_type(&mut self.word.types, &mut self.word.type_sizes, lir::AT_QUOT, self.nominals, span),
            Value::MmioPlace(_) => Ok(lir::TY_MMIO),
            Value::Ptr { mutable: false, .. } => Ok(lir::TY_PTR),
            Value::Ptr { mutable: true, .. } => Ok(lir::TY_PTR_MUT),
            Value::MmioPtr { mutable: false, .. } => Ok(lir::TY_PTR),
            Value::MmioPtr { mutable: true, .. } => Ok(lir::TY_PTR_MUT),
        }
    }
}

pub fn intern_type(
    types: &mut FixedVec<lir::Atom, 64>,
    type_sizes: &mut FixedVec<u32, 64>,
    atom: lir::Atom,
    nominals: &NominalDb,
    span: Span,
) -> Result<lir::TypeId, TcError> {
    for (i, a) in types.iter().enumerate() {
        if *a == atom {
            return Ok(lir::TypeId(i as u8));
        }
    }
    let idx = types.len();
    if idx > u8::MAX as usize {
        return Err(TcError::TooManyTypes { span });
    }
    let size = TypeAtom::new(atom.as_bytes()).and_then(|ty| type_size_bytes(ty, nominals)).unwrap_or(0);
    types.push(atom).map_err(|_| TcError::TypeTableFull { span })?;
    type_sizes.push(size).map_err(|_| TcError::TypeTableFull { span })?;
    Ok(lir::TypeId(idx as u8))
}
