use crate::typecheck::error::TcError;
use crate::typecheck::parse::parse_type_expr;
use crate::typecheck::util::slice_span;
use crate::types::TypeAtom;
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
    /// Sharing class: 0 = main-only, 1 = main+ISR (single-core), 2 = multi-core.
    /// Updated by the cross-context resource-sharing analysis (effect-context-model §6.2).
    pub sharing_class: u8,
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
        if d.kind != DeclKind::Iso && d.kind != DeclKind::Owned {
            continue;
        }
        let name = TypeAtom::new(slice_span(src, d.name))
            .ok_or(TcError::IsoNameInvalid { span: d.name })?;
        types
            .push(name)
            .map_err(|_| TcError::IsoCapacityExceeded { span: d.name })?;
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

/// A word bound to an interrupt vector via `@interrupt(VEC)`.
#[derive(Clone, Copy)]
pub struct IsrBinding {
    /// Name of the bound word.
    pub word_name: TypeAtom,
    /// Vector/shorthand name parsed from `@interrupt(VEC)` source text, e.g. `TIMER0`.
    pub vec_name: TypeAtom,
}

/// Collects all `@interrupt(VEC)` bindings from word declarations.
/// Returns an empty vec if none are found.
pub fn build_isr_bindings(module: &ModuleAst, src: &[u8]) -> FixedVec<IsrBinding, 16> {
    let mut bindings: FixedVec<IsrBinding, 16> = FixedVec::new();
    for d in module.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        for a in d.attrs.iter() {
            let bytes = slice_span(src, *a);
            if let Some(vec_name) = bytes.strip_prefix(b"@interrupt(").and_then(|s| {
                let end = s.iter().position(|&b| b == b')')?;
                TypeAtom::new(&s[..end])
            }) {
                let word_name = TypeAtom::new(slice_span(src, d.name)).unwrap_or(TypeAtom::EMPTY);
                let _ = bindings.push(IsrBinding {
                    word_name,
                    vec_name,
                });
            }
        }
    }
    bindings
}

pub fn build_resource_db(module: &ModuleAst, src: &[u8]) -> Result<ResourceDb, TcError> {
    let mut items: FixedVec<ResourceInfo, 64> = FixedVec::new();
    for d in module.decls.iter() {
        if d.kind != DeclKind::Resource {
            continue;
        }
        let name = TypeAtom::new(slice_span(src, d.name))
            .ok_or(TcError::ResourceNameInvalid { span: d.name })?;
        let ty = if let Some(s) = d.sig {
            parse_type_atom_from_span(src, s).ok_or(TcError::ResourceTypeInvalid { span: s })?
        } else {
            TypeAtom::I64
        };
        items
            .push(ResourceInfo {
                name,
                ty,
                sharing_class: 0,
            })
            .map_err(|_| TcError::ResourceCapacityExceeded { span: d.name })?;
    }
    Ok(ResourceDb { items })
}

pub fn build_nominal_db(module: &ModuleAst, src: &[u8]) -> Result<NominalDb, TcError> {
    let mut structs: FixedVec<StructInfo, 64> = FixedVec::new();
    let mut enums: FixedVec<EnumInfo, 64> = FixedVec::new();

    for sdecl in module.structs.iter() {
        let name = TypeAtom::new(slice_span(src, sdecl.name))
            .ok_or(TcError::StructNameInvalid { span: sdecl.name })?;
        let mut fields: FixedVec<StructFieldInfo, 64> = FixedVec::new();
        for f in sdecl.fields.iter() {
            let fname = TypeAtom::new(slice_span(src, f.name))
                .ok_or(TcError::StructFieldNameInvalid { span: f.name })?;
            for existing in fields.iter() {
                if existing.name == fname {
                    return Err(TcError::StructFieldDuplicate { span: f.name });
                }
            }
            let fty = parse_type_atom_from_span(src, f.ty)
                .ok_or(TcError::StructFieldTypeInvalid { span: f.ty })?;
            fields
                .push(StructFieldInfo {
                    name: fname,
                    ty: fty,
                })
                .map_err(|_| TcError::StructDbCapacity { span: f.name })?;
        }
        structs
            .push(StructInfo { name, fields })
            .map_err(|_| TcError::StructDbCapacity { span: sdecl.name })?;
    }

    for edecl in module.enums.iter() {
        let name = TypeAtom::new(slice_span(src, edecl.name))
            .ok_or(TcError::EnumNameInvalid { span: edecl.name })?;
        let base = match edecl.base {
            Some(s) => {
                parse_type_atom_from_span(src, s).ok_or(TcError::EnumBaseTypeInvalid { span: s })?
            }
            None => TypeAtom::EMPTY,
        };
        let mut variants: FixedVec<EnumVariantInfo, 64> = FixedVec::new();
        for v in edecl.variants.iter() {
            let vname = TypeAtom::new(slice_span(src, v.name))
                .ok_or(TcError::EnumVariantNameInvalid { span: v.name })?;
            for existing in variants.iter() {
                if existing.name == vname {
                    return Err(TcError::EnumVariantDuplicate { span: v.name });
                }
            }
            variants
                .push(EnumVariantInfo {
                    name: vname,
                    value: 0,
                })
                .map_err(|_| TcError::EnumVariantCapacityExceeded { span: v.name })?;
        }
        enums
            .push(EnumInfo {
                name,
                base,
                variants,
            })
            .map_err(|_| TcError::EnumVariantCapacityExceeded { span: edecl.name })?;
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

pub fn resource_sharing_class(db: &ResourceDb, name: TypeAtom) -> u8 {
    for r in db.items.iter() {
        if r.name == name {
            return r.sharing_class;
        }
    }
    0
}

/// Mark resources that are reachable from `@interrupt(VEC)` words as shared.
/// Sets `sharing_class = 1` (main + ISR, single-core) for each such resource.
/// This is a simple declaration-level scan: for every ISR word, we scan its
/// body text for any resource name known to the db.
pub fn compute_resource_sharing(module: &ModuleAst, src: &[u8], db: &mut ResourceDb) {
    // Collect ISR word bodies.
    let isr_bodies: FixedVec<Span, 64> = {
        let mut bodies = FixedVec::new();
        for d in module.decls.iter() {
            if d.kind != DeclKind::Word {
                continue;
            }
            let is_isr = d
                .attrs
                .iter()
                .any(|a| slice_span(src, *a).starts_with(b"@interrupt("));
            if is_isr {
                if let Some(body) = d.body {
                    let _ = bodies.push(body);
                }
            }
        }
        bodies
    };
    if isr_bodies.is_empty() {
        return; // no ISRs → nothing is shared
    }

    // For each resource, check if its name appears in any ISR body.
    for i in 0..db.items.len() {
        let rname = db.items.get(i).map(|r| r.name).unwrap_or(TypeAtom::EMPTY);
        if rname.as_bytes().is_empty() {
            continue;
        }
        let name_bytes = rname.as_bytes();
        for body_span in isr_bodies.iter() {
            let body_slice = &src[body_span.start..body_span.end];
            if body_slice
                .windows(name_bytes.len())
                .any(|w| w == name_bytes)
            {
                if let Some(r) = db.items.get_mut(i) {
                    r.sharing_class = 1;
                }
                break;
            }
        }
    }
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
