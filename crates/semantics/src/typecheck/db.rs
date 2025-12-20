use crate::types::TypeAtom;
use crate::typecheck::error::TcError;
use crate::typecheck::util::slice_span;
use crate::typecheck::parse::parse_type_expr;
use frontend::fixed::FixedVec;
use frontend::parse::{DeclKind, ModuleAst};
use frontend::span::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubtypeInfo {
    pub name: TypeAtom,
    pub base: TypeAtom,
    pub min: i64,
    pub max: i64,
}

#[derive(Clone, Copy)]
pub struct ResourceInfo {
    pub name: TypeAtom,
    pub ty: TypeAtom,
}

pub struct ResourceDb {
    pub items: FixedVec<ResourceInfo, 64>,
}

#[derive(Clone, Copy)]
pub struct StructFieldInfo {
    pub name: TypeAtom,
    pub ty: TypeAtom,
}

pub struct StructInfo {
    pub name: TypeAtom,
    pub fields: FixedVec<StructFieldInfo, 64>,
}

pub struct EnumVariantInfo {
    pub name: TypeAtom,
    pub value: i64,
}

pub struct EnumInfo {
    pub name: TypeAtom,
    pub base: TypeAtom,
    pub variants: FixedVec<EnumVariantInfo, 64>,
}

pub struct NominalDb {
    pub structs: FixedVec<StructInfo, 64>,
    pub enums: FixedVec<EnumInfo, 64>,
}

pub struct IsoDb {
    pub types: FixedVec<TypeAtom, 64>,
}

pub fn build_iso_db(module: &ModuleAst, src: &[u8]) -> Result<IsoDb, TcError> {
    let mut types: FixedVec<TypeAtom, 64> = FixedVec::new();
    for d in module.decls.iter() {
        if d.kind != DeclKind::Iso {
            continue;
        }
        let name = TypeAtom::new(slice_span(src, d.name)).ok_or(TcError { code: 3740, span: d.name })?;
        let _ = types.push(name).map_err(|_| TcError { code: 3741, span: d.name })?;
    }
    Ok(IsoDb { types })
}

pub fn is_iso_type(iso: &IsoDb, ty: TypeAtom) -> bool {
    for t in iso.types.iter() {
        if *t == ty {
            return true;
        }
    }
    false
}

pub fn build_resource_db(module: &ModuleAst, src: &[u8]) -> Result<ResourceDb, TcError> {
    let mut items: FixedVec<ResourceInfo, 64> = FixedVec::new();
    for d in module.decls.iter() {
        if d.kind != DeclKind::Resource {
            continue;
        }
        let name = TypeAtom::new(slice_span(src, d.name)).ok_or(TcError { code: 3520, span: d.name })?;
        let ty = if let Some(s) = d.sig {
            parse_type_atom_from_span(src, s).ok_or(TcError { code: 3521, span: s })?
        } else {
            TypeAtom::new(b"i64").unwrap()
        };
        let _ = items.push(ResourceInfo { name, ty }).map_err(|_| TcError { code: 3522, span: d.name })?;
    }
    Ok(ResourceDb { items })
}

pub fn build_nominal_db(module: &ModuleAst, src: &[u8]) -> Result<NominalDb, TcError> {
    let mut structs: FixedVec<StructInfo, 64> = FixedVec::new();
    let mut enums: FixedVec<EnumInfo, 64> = FixedVec::new();

    for sdecl in module.structs.iter() {
        let name = TypeAtom::new(slice_span(src, sdecl.name)).ok_or(TcError { code: 3710, span: sdecl.name })?;
        let mut fields: FixedVec<StructFieldInfo, 64> = FixedVec::new();
        for f in sdecl.fields.iter() {
            let fname = TypeAtom::new(slice_span(src, f.name)).ok_or(TcError { code: 3711, span: f.name })?;
            for prev in fields.iter() {
                if prev.name == fname {
                    return Err(TcError { code: 3712, span: f.name });
                }
            }
            let fty = parse_type_atom_from_span(src, f.ty).ok_or(TcError { code: 3713, span: f.ty })?;
            let _ = fields
                .push(StructFieldInfo { name: fname, ty: fty })
                .map_err(|_| TcError { code: 3714, span: f.name })?;
        }
        structs.push(StructInfo { name, fields }).map_err(|_| TcError { code: 3714, span: sdecl.name })?;
    }

    for edecl in module.enums.iter() {
        let name = TypeAtom::new(slice_span(src, edecl.name)).ok_or(TcError { code: 3720, span: edecl.name })?;
        let base = if let Some(s) = edecl.base {
            parse_type_atom_from_span(src, s).ok_or(TcError { code: 3721, span: s })?
        } else {
            TypeAtom::new(b"i64").unwrap()
        };
        let mut variants: FixedVec<EnumVariantInfo, 64> = FixedVec::new();
        for v in edecl.variants.iter() {
            let vname = TypeAtom::new(slice_span(src, v.name)).ok_or(TcError { code: 3722, span: v.name })?;
            for prev in variants.iter() {
                if prev.name == vname {
                    return Err(TcError { code: 3723, span: v.name });
                }
            }
            let _ = variants
                .push(EnumVariantInfo { name: vname, value: v.value })
                .map_err(|_| TcError { code: 3724, span: v.name })?;
        }
        enums.push(EnumInfo { name, base, variants }).map_err(|_| TcError { code: 3724, span: edecl.name })?;
    }

    Ok(NominalDb { structs, enums })
}

fn parse_type_atom_from_span(src: &[u8], span: Span) -> Option<TypeAtom> {
    let slice = &src[span.start..span.end];
    let (atom, next) = parse_type_expr(slice, 0)?;
    // allow trailing whitespace
    let mut i = next;
    while i < slice.len() && matches!(slice[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    if i != slice.len() {
        return None;
    }
    Some(atom)
}

pub fn resource_ty(db: &ResourceDb, name: TypeAtom) -> Option<TypeAtom> {
    for r in db.items.iter() {
        if r.name == name {
            return Some(r.ty);
        }
    }
    None
}

pub fn struct_field_ty(db: &NominalDb, struct_ty: TypeAtom, field: TypeAtom) -> Option<TypeAtom> {
    for s in db.structs.iter() {
        if s.name != struct_ty {
            continue;
        }
        for f in s.fields.iter() {
            if f.name == field {
                return Some(f.ty);
            }
        }
        return None;
    }
    None
}

pub fn enum_variant_value(db: &NominalDb, enum_ty: TypeAtom, variant: TypeAtom) -> Option<i64> {
    for e in db.enums.iter() {
        if e.name != enum_ty {
            continue;
        }
        for v in e.variants.iter() {
            if v.name == variant {
                return Some(v.value);
            }
        }
        return None;
    }
    None
}
