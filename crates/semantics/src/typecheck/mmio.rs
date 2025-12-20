use crate::types::TypeAtom;
use crate::typecheck::error::TcError;
use crate::typecheck::util::{parse_u32_any, slice_span};
use frontend::fixed::FixedVec;
use frontend::lex::Lexer;
use frontend::parse::{DeclKind, ModuleAst};
use frontend::span::Span;
use frontend::token::TokenKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Ro,
    Wo,
    Rw,
    W1c,
    W1s,
    Rc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioFieldInfo {
    pub name: TypeAtom,
    pub lo: u8,
    pub hi: u8,
    pub ty: TypeAtom,
    pub access: AccessMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioMapDecl {
    pub name: TypeAtom,
    pub body: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioInstance {
    pub name: TypeAtom,
    pub map: TypeAtom,
    pub base_addr: u64,
}

pub struct MmioDb {
    pub maps: FixedVec<MmioMapDecl, 16>,
    pub instances: FixedVec<MmioInstance, 64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioRegInfo {
    pub offset: u32,
    pub reg: TypeAtom,
    pub reg_ty: TypeAtom,
    pub access: AccessMode,
    pub volatile: bool,
    pub array_len: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioResolvedReg {
    pub map: TypeAtom,
    pub reg: TypeAtom,
    pub reg_ty: TypeAtom,
    pub access: AccessMode,
    pub volatile: bool,
    pub addr: u64,
    pub array_len: Option<u32>,
    // The original source span of the whole place (e.g. `gpio.OUT_SET`).
    pub place_span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioResolvedField {
    pub map: TypeAtom,
    pub reg: TypeAtom,
    pub reg_ty: TypeAtom,
    pub reg_access: AccessMode,
    pub field: MmioFieldInfo,
    pub volatile: bool,
    pub addr: u64,
    pub array_len: Option<u32>,
    pub place_span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmioResolved {
    Reg(MmioResolvedReg),
    Field(MmioResolvedField),
}

pub fn access_can_read(access: AccessMode) -> bool {
    !matches!(access, AccessMode::Wo)
}

pub fn access_can_write(access: AccessMode) -> bool {
    !matches!(access, AccessMode::Ro)
}

pub fn field_mask_shift(field: &MmioFieldInfo) -> (u64, u8) {
    let lo = field.lo as u32;
    let hi = field.hi as u32;
    let width = hi.saturating_sub(lo).saturating_add(1).min(64);
    let mask = if width >= 64 {
        !0u64
    } else {
        (1u64 << width) - 1
    };
    (mask << lo, field.lo)
}

pub fn mmio_type_width_bytes(ty: &[u8]) -> Option<u32> {
    match ty {
        b"bool" | b"u8" | b"i8" => Some(1),
        b"u16" | b"i16" => Some(2),
        b"u32" | b"i32" => Some(4),
        b"u64" | b"i64" => Some(8),
        _ => None,
    }
}

pub fn build_mmio_db(module: &ModuleAst, src: &[u8]) -> Result<MmioDb, TcError> {
    let mut db = MmioDb {
        maps: FixedVec::new(),
        instances: FixedVec::new(),
    };

    for inst in module.instances.iter() {
        let Some(name) = TypeAtom::new(slice_span(src, inst.name)) else {
            continue;
        };
        let Some(map) = TypeAtom::new(slice_span(src, inst.map)) else {
            continue;
        };
        let base_addr = parse_u32_any(slice_span(src, inst.base_addr)).unwrap_or(0) as u64;
        let _ = db.instances.push(MmioInstance { name, map, base_addr });
    }

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::RegisterMap {
            continue;
        }
        let Some(body) = decl.body else {
            continue;
        };
        let Some(map_name) = TypeAtom::new(slice_span(src, decl.name)) else {
            continue;
        };
        validate_regmap_body(src, body)?;
        let _ = db.maps.push(MmioMapDecl { name: map_name, body });
    }

    Ok(db)
}

fn validate_regmap_body(src: &[u8], body: Span) -> Result<(), TcError> {
    let slice = &src[body.start..body.end];
    let mut lex = Lexer::new(slice);
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            break;
        }
        if tok.kind != TokenKind::Number {
            continue;
        }

        let offset = parse_u32_any(&slice[tok.span.start..tok.span.end]).ok_or(TcError {
            code: 3615,
            span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
        })?;

        let name_tok = lex.next();
        if name_tok.kind != TokenKind::Ident {
            return Err(TcError { code: 3616, span: Span::new(body.start + name_tok.span.start, body.start + name_tok.span.end) });
        }

        let ty_tok = lex.next();
        if ty_tok.kind != TokenKind::Ident {
            return Err(TcError { code: 3618, span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end) });
        }
        let ty_bytes = &slice[ty_tok.span.start..ty_tok.span.end];
        let Some(width) = mmio_type_width_bytes(ty_bytes) else {
            return Err(TcError { code: 3612, span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end) });
        };
        if offset % width != 0 {
            return Err(TcError { code: 3611, span: Span::new(body.start + tok.span.start, body.start + tok.span.end) });
        }

        let access_tok = lex.next();
        if access_tok.kind != TokenKind::Ident {
            return Err(TcError { code: 3620, span: Span::new(body.start + access_tok.span.start, body.start + access_tok.span.end) });
        }
        if parse_access_mode(&slice[access_tok.span.start..access_tok.span.end]).is_none() {
            return Err(TcError { code: 3621, span: Span::new(body.start + access_tok.span.start, body.start + access_tok.span.end) });
        }

        // Optional `volatile`
        let mut probe = lex;
        let next = probe.next();
        if next.kind == TokenKind::Ident && &slice[next.span.start..next.span.end] == b"volatile" {
            lex = probe;
        }

        // Optional field block `{ ... }`: validate ranges and access tokens.
        let mut probe2 = lex;
        let maybe_lbrace = probe2.next();
        if maybe_lbrace.kind == TokenKind::PunctLBrace {
            lex = probe2;
            loop {
                let ftok = lex.next();
                match ftok.kind {
                    TokenKind::PunctRBrace => break,
                    TokenKind::Eof => return Err(TcError { code: 3622, span: Span::new(body.start + maybe_lbrace.span.start, body.start + maybe_lbrace.span.end) }),
                    TokenKind::Ident => {
                        let lo_tok = lex.next();
                        if lo_tok.kind != TokenKind::Number {
                            return Err(TcError { code: 3624, span: Span::new(body.start + lo_tok.span.start, body.start + lo_tok.span.end) });
                        }
                        let lo = parse_u32_any(&slice[lo_tok.span.start..lo_tok.span.end]).ok_or(TcError {
                            code: 3625,
                            span: Span::new(body.start + lo_tok.span.start, body.start + lo_tok.span.end),
                        })?;
                        let mut hi = lo;
                        let mut probe3 = lex;
                        if probe3.next().kind == TokenKind::PunctDblDot {
                            let hi_tok = probe3.next();
                            if hi_tok.kind != TokenKind::Number {
                                return Err(TcError { code: 3626, span: Span::new(body.start + hi_tok.span.start, body.start + hi_tok.span.end) });
                            }
                            hi = parse_u32_any(&slice[hi_tok.span.start..hi_tok.span.end]).ok_or(TcError {
                                code: 3627,
                                span: Span::new(body.start + hi_tok.span.start, body.start + hi_tok.span.end),
                            })?;
                            lex = probe3;
                        }
                        let hi = hi.max(lo);
                        let bit_limit = (width * 8) as u32;
                        if hi >= bit_limit {
                            return Err(TcError { code: 3617, span: Span::new(body.start + lo_tok.span.start, body.start + lo_tok.span.end) });
                        }
                        let fty_tok = lex.next();
                        if fty_tok.kind != TokenKind::Ident {
                            return Err(TcError { code: 3628, span: Span::new(body.start + fty_tok.span.start, body.start + fty_tok.span.end) });
                        }
                        let faccess_tok = lex.next();
                        if faccess_tok.kind != TokenKind::Ident {
                            return Err(TcError { code: 3630, span: Span::new(body.start + faccess_tok.span.start, body.start + faccess_tok.span.end) });
                        }
                        if parse_access_mode(&slice[faccess_tok.span.start..faccess_tok.span.end]).is_none() {
                            return Err(TcError { code: 3631, span: Span::new(body.start + faccess_tok.span.start, body.start + faccess_tok.span.end) });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn parse_access_mode(bytes: &[u8]) -> Option<AccessMode> {
    match bytes {
        b"ro" => Some(AccessMode::Ro),
        b"wo" => Some(AccessMode::Wo),
        b"rw" => Some(AccessMode::Rw),
        b"w1c" => Some(AccessMode::W1c),
        b"w1s" => Some(AccessMode::W1s),
        b"rc" => Some(AccessMode::Rc),
        _ => None,
    }
}

fn parse_name_array(token: &[u8]) -> (&[u8], Option<u32>) {
    // NAME[40]
    let mut i = 0usize;
    while i < token.len() {
        if token[i] == b'\'' {
            let base = &token[..i];
            let mut j = i + 1;
            while j < token.len() && token[j] != b'.' {
                j += 1;
            }
            if j > i + 1 {
                return (base, parse_u32_any(&token[i + 1..j]));
            }
            return (token, None);
        }
        if token[i] == b'[' {
            let base = &token[..i];
            let mut j = i + 1;
            while j < token.len() && token[j] != b']' {
                j += 1;
            }
            if j < token.len() && token[j] == b']' {
                return (base, parse_u32_any(&token[i + 1..j]));
            }
            return (token, None);
        }
        i += 1;
    }
    (token, None)
}

fn find_instance(db: &MmioDb, name: TypeAtom) -> Option<MmioInstance> {
    for inst in db.instances.iter() {
        if inst.name == name {
            return Some(*inst);
        }
    }
    None
}

fn find_map_decl(db: &MmioDb, name: TypeAtom) -> Option<MmioMapDecl> {
    for m in db.maps.iter() {
        if m.name == name {
            return Some(*m);
        }
    }
    None
}

pub fn scan_regmap_for_reg(
    src: &[u8],
    body: Span,
    want_reg: TypeAtom,
    want_field: Option<TypeAtom>,
    place_span: Span,
    _map_name: TypeAtom,
) -> Result<Option<(MmioRegInfo, Option<MmioFieldInfo>)>, TcError> {
    let slice = &src[body.start..body.end];
    let mut lex = Lexer::new(slice);
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            return Ok(None);
        }
        if tok.kind != TokenKind::Number {
            continue;
        }

        let offset = parse_u32_any(&slice[tok.span.start..tok.span.end]).ok_or(TcError {
            code: 3615,
            span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
        })?;

        let name_tok = lex.next();
        if name_tok.kind != TokenKind::Ident {
            return Err(TcError { code: 3616, span: place_span });
        }
        let name_bytes = &slice[name_tok.span.start..name_tok.span.end];
        let (base_name, array_len) = parse_name_array(name_bytes);
        let Some(reg_name) = TypeAtom::new(base_name) else {
            return Err(TcError { code: 3603, span: place_span });
        };

        let ty_tok = lex.next();
        if ty_tok.kind != TokenKind::Ident {
            return Err(TcError { code: 3618, span: place_span });
        }
        let ty_bytes = &slice[ty_tok.span.start..ty_tok.span.end];
        let Some(reg_ty) = TypeAtom::new(ty_bytes) else {
            return Err(TcError { code: 3619, span: place_span });
        };
        let Some(width) = mmio_type_width_bytes(ty_bytes) else {
            return Err(TcError { code: 3612, span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end) });
        };
        if offset % width != 0 {
            return Err(TcError { code: 3611, span: Span::new(body.start + tok.span.start, body.start + tok.span.end) });
        }

        let access_tok = lex.next();
        if access_tok.kind != TokenKind::Ident {
            return Err(TcError { code: 3620, span: place_span });
        }
        let access = parse_access_mode(&slice[access_tok.span.start..access_tok.span.end]).ok_or(TcError { code: 3621, span: place_span })?;

        let mut volatile = false;
        let mut probe = lex;
        let next = probe.next();
        if next.kind == TokenKind::Ident && &slice[next.span.start..next.span.end] == b"volatile" {
            volatile = true;
            lex = probe;
        }

        // Optional field block.
        let mut field_found: Option<MmioFieldInfo> = None;
        let mut probe2 = lex;
        let maybe_lbrace = probe2.next();
        if maybe_lbrace.kind == TokenKind::PunctLBrace {
            lex = probe2;
            loop {
                let ftok = lex.next();
                match ftok.kind {
                    TokenKind::PunctRBrace => break,
                    TokenKind::Eof => return Err(TcError { code: 3622, span: place_span }),
                    TokenKind::Ident => {
                        let fname_bytes = &slice[ftok.span.start..ftok.span.end];
                        let Some(fname) = TypeAtom::new(fname_bytes) else {
                            return Err(TcError { code: 3623, span: place_span });
                        };

                        let lo_tok = lex.next();
                        if lo_tok.kind != TokenKind::Number {
                            return Err(TcError { code: 3624, span: place_span });
                        }
                        let lo = parse_u32_any(&slice[lo_tok.span.start..lo_tok.span.end]).ok_or(TcError { code: 3625, span: place_span })?;
                        let mut hi = lo;
                        let mut probe3 = lex;
                        if probe3.next().kind == TokenKind::PunctDblDot {
                            let hi_tok = probe3.next();
                            if hi_tok.kind != TokenKind::Number {
                                return Err(TcError { code: 3626, span: place_span });
                            }
                            hi = parse_u32_any(&slice[hi_tok.span.start..hi_tok.span.end]).ok_or(TcError { code: 3627, span: place_span })?;
                            lex = probe3;
                        }
                        let hi = hi.max(lo);
                        if hi >= (width * 8) {
                            return Err(TcError { code: 3617, span: place_span });
                        }

                        let fty_tok = lex.next();
                        if fty_tok.kind != TokenKind::Ident {
                            return Err(TcError { code: 3628, span: place_span });
                        }
                        let Some(field_ty) = TypeAtom::new(&slice[fty_tok.span.start..fty_tok.span.end]) else {
                            return Err(TcError { code: 3629, span: place_span });
                        };

                        let faccess_tok = lex.next();
                        if faccess_tok.kind != TokenKind::Ident {
                            return Err(TcError { code: 3630, span: place_span });
                        }
                        let faccess = parse_access_mode(&slice[faccess_tok.span.start..faccess_tok.span.end]).ok_or(TcError { code: 3631, span: place_span })?;

                        if let Some(want) = want_field {
                            if fname == want && field_found.is_none() {
                                field_found = Some(MmioFieldInfo {
                                    name: fname,
                                    lo: lo.min(255) as u8,
                                    hi: hi.min(255) as u8,
                                    ty: field_ty,
                                    access: faccess,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if reg_name != want_reg {
            continue;
        }

        if let Some(_want) = want_field {
            if field_found.is_none() {
                return Err(TcError { code: 3607, span: place_span });
            }
            return Ok(Some((
                MmioRegInfo {
                    offset,
                    reg: reg_name,
                    reg_ty,
                    access,
                    volatile,
                    array_len,
                },
                field_found,
            )));
        }
        return Ok(Some((
            MmioRegInfo {
                offset,
                reg: reg_name,
                reg_ty,
                access,
                volatile,
                array_len,
            },
            None,
        )));
    }
}

fn split_segments<'a>(name: &'a [u8], out: &mut [&'a [u8]; 8]) -> usize {
    let mut count = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i <= name.len() {
        if i == name.len() || name[i] == b'.' {
            if count < out.len() {
                out[count] = &name[start..i];
                count += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    count
}

pub fn resolve_mmio_place(db: &MmioDb, src: &[u8], name: &[u8], place_span: Span) -> Result<Option<MmioResolved>, TcError> {
    let mut segs: [&[u8]; 8] = [&[]; 8];
    let seg_len = split_segments(name, &mut segs);
    if seg_len < 2 {
        return Ok(None);
    }

    let Some(inst_name) = TypeAtom::new(segs[0]) else {
        return Ok(None);
    };
    let Some(inst) = find_instance(db, inst_name) else {
        return Ok(None);
    };
    let Some(map_decl) = find_map_decl(db, inst.map) else {
        return Err(TcError { code: 3602, span: place_span });
    };

    let (reg_base, reg_idx) = parse_name_array(segs[1]);
    let Some(reg_name) = TypeAtom::new(reg_base) else {
        return Err(TcError { code: 3603, span: place_span });
    };
    let want_field = if seg_len == 3 {
        Some(TypeAtom::new(segs[2]).ok_or(TcError { code: 3607, span: place_span })?)
    } else {
        None
    };
    if seg_len > 3 {
        return Err(TcError { code: 3600, span: place_span });
    }

    let reg_info = scan_regmap_for_reg(src, map_decl.body, reg_name, want_field, place_span, map_decl.name)?;
    let Some((reg_info, field_info)) = reg_info else {
        return Err(TcError { code: 3603, span: place_span });
    };

    if let Some(n) = reg_info.array_len {
        if let Some(idx) = reg_idx {
            if idx >= n {
                return Err(TcError { code: 3604, span: place_span });
            }
        }
    } else if reg_idx.is_some() {
        return Err(TcError { code: 3606, span: place_span });
    }

    if let Some(field) = field_info {
        let width = mmio_type_width_bytes(reg_info.reg_ty.as_bytes()).unwrap_or(1) as u64;
        let idx = reg_idx.unwrap_or(0) as u64;
        let addr = inst.base_addr.wrapping_add(reg_info.offset as u64).wrapping_add(idx.wrapping_mul(width));
        Ok(Some(MmioResolved::Field(MmioResolvedField {
            map: map_decl.name,
            reg: reg_info.reg,
            reg_ty: reg_info.reg_ty,
            reg_access: reg_info.access,
            field,
            volatile: reg_info.volatile,
            addr,
            array_len: reg_info.array_len,
            place_span,
        })))
    } else {
        let width = mmio_type_width_bytes(reg_info.reg_ty.as_bytes()).unwrap_or(1) as u64;
        let idx = reg_idx.unwrap_or(0) as u64;
        let addr = inst.base_addr.wrapping_add(reg_info.offset as u64).wrapping_add(idx.wrapping_mul(width));
        Ok(Some(MmioResolved::Reg(MmioResolvedReg {
            map: map_decl.name,
            reg: reg_info.reg,
            reg_ty: reg_info.reg_ty,
            access: reg_info.access,
            volatile: reg_info.volatile,
            addr,
            array_len: reg_info.array_len,
            place_span,
        })))
    }
}
