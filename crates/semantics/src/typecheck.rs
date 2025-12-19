use crate::types::{SigParseError, TypeAtom, WordEntry, WordSig};
use frontend::{
    fixed::FixedVec,
    lex::Lexer,
    parse::{DeclAst, DeclKind, ModuleAst},
    span::Span,
    token::TokenKind,
};
use ir as lir;

pub trait Output {
    fn write(&mut self, bytes: &[u8]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcError {
    pub code: u32,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Value {
    Plain(TypeAtom),
    Scoped { ty: TypeAtom, scope: u16 },
    Resource(TypeAtom),
    Quot(Span),
    MmioPlace(MmioResolved),
    Ptr { ty: TypeAtom, mutable: bool },
    MmioPtr {
        reg: MmioResolvedReg,
        mutable: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksMode {
    Off,
    Contracts,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubtypeInfo {
    pub name: TypeAtom,
    pub base: TypeAtom,
    pub min: i64,
    pub max: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AccessMode {
    Ro,
    Wo,
    Rw,
    W1c,
    W1s,
    Rc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmioFieldInfo {
    name: TypeAtom,
    lo: u8,
    hi: u8,
    ty: TypeAtom,
    access: AccessMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmioMapDecl {
    name: TypeAtom,
    body: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmioInstance {
    name: TypeAtom,
    map: TypeAtom,
    base_addr: u64,
}

struct MmioDb {
    maps: FixedVec<MmioMapDecl, 16>,
    instances: FixedVec<MmioInstance, 64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmioRegInfo {
    offset: u32,
    reg: TypeAtom,
    reg_ty: TypeAtom,
    access: AccessMode,
    volatile: bool,
    array_len: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmioResolvedReg {
    map: TypeAtom,
    reg: TypeAtom,
    reg_ty: TypeAtom,
    access: AccessMode,
    volatile: bool,
    addr: u64,
    // The original source span of the whole place (e.g. `gpio.OUT_SET`).
    place_span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmioResolvedField {
    map: TypeAtom,
    reg: TypeAtom,
    reg_ty: TypeAtom,
    reg_access: AccessMode,
    field: MmioFieldInfo,
    volatile: bool,
    addr: u64,
    place_span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MmioResolved {
    Reg(MmioResolvedReg),
    Field(MmioResolvedField),
}

pub fn parse_word_sig(src: &[u8], sig_span: Span) -> Result<WordSig, SigParseError> {
    let mut sig = WordSig::empty();
    let mut in_phase = true;

    let slice = &src[sig_span.start..sig_span.end];
    let mut i = 0usize;
    while i < slice.len() {
        // whitespace
        while i < slice.len() && matches!(slice[i], b' ' | b'\n' | b'\r' | b'\t') {
            i += 1;
        }
        if i >= slice.len() {
            break;
        }
        // parens are just delimiters
        if slice[i] == b'(' || slice[i] == b')' {
            i += 1;
            continue;
        }
        // "--" separator
        if i + 1 < slice.len() && slice[i] == b'-' && slice[i + 1] == b'-' {
            in_phase = false;
            i += 2;
            continue;
        }

        let start = i;
        let (atom, next) = parse_type_expr(slice, i).ok_or(SigParseError {
            code: 3100,
            span: Span::new(sig_span.start + start, sig_span.start + core::cmp::min(start + 1, slice.len())),
        })?;
        i = next;

        if in_phase {
            let idx = sig.in_len as usize;
            if idx >= sig.inputs.len() {
                return Err(SigParseError { code: 3102, span: sig_span });
            }
            sig.inputs[idx] = atom;
            sig.in_len += 1;
        } else {
            let idx = sig.out_len as usize;
            if idx >= sig.outputs.len() {
                return Err(SigParseError { code: 3103, span: sig_span });
            }
            sig.outputs[idx] = atom;
            sig.out_len += 1;
        }
    }

    Ok(sig)
}

fn parse_type_expr(slice: &[u8], mut i: usize) -> Option<(TypeAtom, usize)> {
    // skip ws
    while i < slice.len() && matches!(slice[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    if i >= slice.len() {
        return None;
    }
    let ident_start = i;
    while i < slice.len() {
        let b = slice[i];
        if matches!(b, b'(' | b')' | b',' | b' ' | b'\n' | b'\r' | b'\t') {
            break;
        }
        i += 1;
    }
    if i == ident_start {
        return None;
    }
    let name = &slice[ident_start..i];
    // skip ws
    while i < slice.len() && matches!(slice[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    if i < slice.len() && slice[i] == b'(' {
        i += 1;
        // type argument
        let (inner, mut j) = parse_type_expr(slice, i)?;
        // skip ws
        while j < slice.len() && matches!(slice[j], b' ' | b'\n' | b'\r' | b'\t') {
            j += 1;
        }
        if name == b"Array" {
            if j >= slice.len() || slice[j] != b',' {
                return None;
            }
            j += 1;
            while j < slice.len() && matches!(slice[j], b' ' | b'\n' | b'\r' | b'\t') {
                j += 1;
            }
            let num_start = j;
            while j < slice.len() && !matches!(slice[j], b')' | b' ' | b'\n' | b'\r' | b'\t') {
                j += 1;
            }
            let n_bytes = &slice[num_start..j];
            let n = parse_u32_any(n_bytes)?;
            while j < slice.len() && matches!(slice[j], b' ' | b'\n' | b'\r' | b'\t') {
                j += 1;
            }
            if j >= slice.len() || slice[j] != b')' {
                return None;
            }
            j += 1;
            let mut buf = [0u8; 32];
            let mut k = 0usize;
            k = push_bytes(&mut buf, k, b"Array(")?;
            k = push_bytes(&mut buf, k, inner.as_bytes())?;
            k = push_bytes(&mut buf, k, b",")?;
            k = push_u32_dec(&mut buf, k, n)?;
            k = push_bytes(&mut buf, k, b")")?;
            let atom = TypeAtom::new(&buf[..k])?;
            return Some((atom, j));
        }
        if name == b"Slice" || name == b"SliceMut" {
            while j < slice.len() && matches!(slice[j], b' ' | b'\n' | b'\r' | b'\t') {
                j += 1;
            }
            if j >= slice.len() || slice[j] != b')' {
                return None;
            }
            j += 1;
            let mut buf = [0u8; 32];
            let mut k = 0usize;
            k = push_bytes(&mut buf, k, name)?;
            k = push_bytes(&mut buf, k, b"(")?;
            k = push_bytes(&mut buf, k, inner.as_bytes())?;
            k = push_bytes(&mut buf, k, b")")?;
            let atom = TypeAtom::new(&buf[..k])?;
            return Some((atom, j));
        }
        if name == b"Chan" {
            while j < slice.len() && matches!(slice[j], b' ' | b'\n' | b'\r' | b'\t') {
                j += 1;
            }
            if j >= slice.len() || slice[j] != b')' {
                return None;
            }
            j += 1;
            let mut buf = [0u8; 32];
            let mut k = 0usize;
            k = push_bytes(&mut buf, k, b"Chan(")?;
            k = push_bytes(&mut buf, k, inner.as_bytes())?;
            k = push_bytes(&mut buf, k, b")")?;
            let atom = TypeAtom::new(&buf[..k])?;
            return Some((atom, j));
        }
        None
    } else {
        let atom = TypeAtom::new(name)?;
        Some((atom, i))
    }
}

fn push_bytes(buf: &mut [u8; 32], mut at: usize, bytes: &[u8]) -> Option<usize> {
    if at + bytes.len() > buf.len() {
        return None;
    }
    for &b in bytes {
        buf[at] = b;
        at += 1;
    }
    Some(at)
}

fn push_u32_dec(buf: &mut [u8; 32], at: usize, mut v: u32) -> Option<usize> {
    let mut tmp = [0u8; 10];
    let mut n = 0usize;
    if v == 0 {
        tmp[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < tmp.len() {
            tmp[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        tmp[..n].reverse();
    }
    push_bytes(buf, at, &tmp[..n])
}

fn parse_u32_any(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let mut base = 10u32;
    if bytes.len() >= 2 && bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X') {
        base = 16;
        i = 2;
    }
    let mut v: u32 = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'_' {
            i += 1;
            continue;
        }
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'f' if base == 16 => 10 + (b - b'a') as u32,
            b'A'..=b'F' if base == 16 => 10 + (b - b'A') as u32,
            _ => return None,
        };
        v = v.checked_mul(base)?;
        v = v.checked_add(digit)?;
        i += 1;
    }
    Some(v)
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

fn mmio_type_width_bytes(ty: &[u8]) -> Option<u32> {
    match ty {
        b"bool" | b"u8" | b"i8" => Some(1),
        b"u16" | b"i16" => Some(2),
        b"u32" | b"i32" => Some(4),
        b"u64" | b"i64" => Some(8),
        _ => None,
    }
}

fn build_mmio_db(module: &ModuleAst, src: &[u8]) -> Result<MmioDb, TcError> {
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

fn scan_regmap_for_reg(
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

fn resolve_mmio_place(db: &MmioDb, src: &[u8], name: &[u8], place_span: Span) -> Result<Option<MmioResolved>, TcError> {
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
        let Some(idx) = reg_idx else {
            return Err(TcError { code: 3605, span: place_span });
        };
        if idx >= n {
            return Err(TcError { code: 3604, span: place_span });
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
            place_span,
        })))
    }
}

fn access_can_read(access: AccessMode) -> bool {
    !matches!(access, AccessMode::Wo)
}

fn access_can_write(access: AccessMode) -> bool {
    !matches!(access, AccessMode::Ro)
}

fn field_mask_shift(field: &MmioFieldInfo) -> (u64, u8) {
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

pub fn emit_ir(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = build_mmio_db(module, src)?;
    let resources = build_resource_db(module, src)?;
    let nominals = build_nominal_db(module, src)?;
    let iso = build_iso_db(module, src)?;

    out.write(b"module ");
    out.write(slice_span(src, module.name));
    out.write(b"\n");

    struct IrOut<'a, O: Output>(&'a mut O);
    impl<'a, O: Output> lir::Output for IrOut<'a, O> {
        fn write(&mut self, bytes: &[u8]) {
            self.0.write(bytes)
        }
    }
    let mut ir_out = IrOut(out);

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError { code: 3200, span: decl.name });
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span })?;
        if decl.body.is_none() {
            ir_out.0.write(b"word ");
            ir_out.0.write(lir_atom_lossy(slice_span(src, decl.name)).as_bytes());
            ir_out.0.write(b" ");
            // Keep legacy behavior: declarations without bodies don't need IR blocks.
            // Still show the signature so `--emit=ir` is useful on `.def` files.
            {
                // Minimal signature printer for the IR dump.
                ir_out.0.write(b"( ");
                for i in 0..(sig.in_len as usize) {
                    if i != 0 {
                        ir_out.0.write(b" ");
                    }
                    ir_out.0.write(sig.inputs[i].as_bytes());
                }
                ir_out.0.write(b" --");
                if sig.out_len > 0 {
                    ir_out.0.write(b" ");
                }
                for i in 0..(sig.out_len as usize) {
                    if i != 0 {
                        ir_out.0.write(b" ");
                    }
                    ir_out.0.write(sig.outputs[i].as_bytes());
                }
                ir_out.0.write(b" )\n");
            }
            continue;
        }

        let w = build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig)?;
        lir::verify_word(&w).map_err(|e| TcError { code: e.code, span: e.span })?;
        lir::write_word(&mut ir_out, &w);
    }
    Ok(())
}

pub fn emit_stackcheck(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = build_mmio_db(module, src)?;
    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            continue;
        };
        let Some(body_span) = decl.body else {
            continue;
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span })?;
        out.write(b"word ");
        out.write(slice_span(src, decl.name));
        out.write(b" ");
        // Reuse the legacy pretty-printer (kept for stackcheck output).
        write_sig(out, &sig);
        out.write(b"\n");
        typecheck_word_body(out, src, body_span, &sig, env, subtypes, &mmio, checks, true)?;
    }
    Ok(())
}

fn lir_atom_lossy(bytes: &[u8]) -> lir::Atom {
    lir::Atom::new(bytes).unwrap_or(lir::Atom::new(b"?").unwrap())
}

fn intern_type(types: &mut FixedVec<lir::Atom, 64>, atom: lir::Atom, span: Span) -> Result<lir::TypeId, TcError> {
    for (i, a) in types.iter().enumerate() {
        if *a == atom {
            return Ok(lir::TypeId(i as u8));
        }
    }
    let idx = types.len();
    if idx > u8::MAX as usize {
        return Err(TcError { code: 3907, span });
    }
    types.push(atom).map_err(|_| TcError { code: 3902, span })?;
    Ok(lir::TypeId(idx as u8))
}

pub fn build_ir_words(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
) -> Result<FixedVec<lir::Word, 256>, TcError> {
    let mmio = build_mmio_db(module, src)?;
    let resources = build_resource_db(module, src)?;
    let nominals = build_nominal_db(module, src)?;
    let iso = build_iso_db(module, src)?;
    let mut out: FixedVec<lir::Word, 256> = FixedVec::new();

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        if decl.body.is_none() {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError { code: 3200, span: decl.name });
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span })?;
        let w = build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig)?;
        lir::verify_word(&w).map_err(|e| TcError { code: e.code, span: e.span })?;
        out.push(w).map_err(|_| TcError { code: 3901, span: decl.name })?;
    }
    Ok(out)
}

pub enum ForEachIrError<E> {
    Type(TcError),
    Consumer(E),
}

pub fn for_each_ir_word<E, F>(
    module: &ModuleAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    checks: ChecksMode,
    allow_raw_casts: bool,
    mut f: F,
) -> Result<(), ForEachIrError<E>>
where
    F: FnMut(&lir::Word) -> Result<(), E>,
{
    let mmio = build_mmio_db(module, src).map_err(ForEachIrError::Type)?;
    let resources = build_resource_db(module, src).map_err(ForEachIrError::Type)?;
    let nominals = build_nominal_db(module, src).map_err(ForEachIrError::Type)?;
    let iso = build_iso_db(module, src).map_err(ForEachIrError::Type)?;

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        if decl.body.is_none() {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(ForEachIrError::Type(TcError { code: 3200, span: decl.name }));
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError { code: e.code, span: e.span }).map_err(ForEachIrError::Type)?;
        let w =
            build_ir_word(decl, src, env, subtypes, &mmio, &resources, &nominals, &iso, checks, allow_raw_casts, &sig)
                .map_err(ForEachIrError::Type)?;
        lir::verify_word(&w).map_err(|e| TcError { code: e.code, span: e.span }).map_err(ForEachIrError::Type)?;
        f(&w).map_err(ForEachIrError::Consumer)?;
    }
    Ok(())
}

fn build_ir_word(
    decl: &DeclAst,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    resources: &ResourceDb,
    nominals: &NominalDb,
    iso: &IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
    sig: &WordSig,
) -> Result<lir::Word, TcError> {
    let name = lir_atom_lossy(slice_span(src, decl.name));
    let mut gen = IrWordGen::new(src, env, subtypes, mmio, resources, nominals, iso, checks, allow_raw_casts, sig, name)?;

    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;
    for i in 0..(sig.in_len as usize) {
        stack[sp] = Value::Plain(sig.inputs[i]);
        sp += 1;
    }

    let mut cur = lir::BlockId(0);
    cur = gen.emit_prologue(cur, &mut stack, &mut sp, decl.requires)?;

    if let Some(body_span) = decl.body {
        cur = gen.compile_span(cur, &mut stack, &mut sp, body_span, decl.effect_suspend, true)?;
    }

    if !gen.check_no_scoped_live(&stack, sp) {
        return Err(TcError { code: 3504, span: decl.body.unwrap_or(decl.name) });
    }

    if gen.terminated {
        return Ok(gen.word);
    }

    if sp != sig.out_len as usize {
        return Err(TcError { code: 3220, span: decl.body.unwrap_or(decl.name) });
    }
	    for i in 0..(sig.out_len as usize) {
	        let got = match stack[i] {
	            Value::Plain(t) => t,
	            Value::Scoped { ty, .. } => ty,
	            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	        };
	        if !type_compatible(got, sig.outputs[i], subtypes) {
	            return Err(TcError { code: 3221, span: decl.body.unwrap_or(decl.name) });
	        }
	    }

    cur = gen.emit_epilogue(cur, &mut stack, &mut sp, decl.ensures)?;
    gen.emit_op(cur, lir::OpKind::Ret, decl.body.unwrap_or(decl.name))?;
    Ok(gen.word)
}

struct IrWordGen<'a> {
    src: &'a [u8],
    env: &'a [WordEntry],
    subtypes: &'a [SubtypeInfo],
    mmio: &'a MmioDb,
    resources: &'a ResourceDb,
    nominals: &'a NominalDb,
    iso: &'a IsoDb,
    checks: ChecksMode,
    allow_raw_casts: bool,
    sig: &'a WordSig,

    locals: [TypeAtom; 64],
    local_tys: [TypeAtom; 64],
    local_live: [bool; 64],
    local_scoped: [u16; 64],
    local_len: usize,

    next_scope: u16,
    scope_stack: [u16; 16],
    scope_sp: usize,

    locked_resource: Option<TypeAtom>,

    terminated: bool,
    word: lir::Word,
}

#[derive(Clone, Copy)]
struct ResourceInfo {
    name: TypeAtom,
    ty: TypeAtom,
}

struct ResourceDb {
    items: FixedVec<ResourceInfo, 64>,
}

struct StructFieldInfo {
    name: TypeAtom,
    ty: TypeAtom,
}

struct StructInfo {
    name: TypeAtom,
    fields: FixedVec<StructFieldInfo, 64>,
}

struct EnumVariantInfo {
    name: TypeAtom,
    value: i64,
}

struct EnumInfo {
    name: TypeAtom,
    base: TypeAtom,
    variants: FixedVec<EnumVariantInfo, 64>,
}

struct NominalDb {
    structs: FixedVec<StructInfo, 64>,
    enums: FixedVec<EnumInfo, 64>,
}

struct IsoDb {
    types: FixedVec<TypeAtom, 64>,
}

fn build_iso_db(module: &ModuleAst, src: &[u8]) -> Result<IsoDb, TcError> {
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

fn is_iso_type(iso: &IsoDb, ty: TypeAtom) -> bool {
    for t in iso.types.iter() {
        if *t == ty {
            return true;
        }
    }
    false
}

fn build_resource_db(module: &ModuleAst, src: &[u8]) -> Result<ResourceDb, TcError> {
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

fn build_nominal_db(module: &ModuleAst, src: &[u8]) -> Result<NominalDb, TcError> {
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

fn resource_ty(db: &ResourceDb, name: TypeAtom) -> Option<TypeAtom> {
    for r in db.items.iter() {
        if r.name == name {
            return Some(r.ty);
        }
    }
    None
}

fn struct_field_ty(db: &NominalDb, struct_ty: TypeAtom, field: TypeAtom) -> Option<TypeAtom> {
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

fn enum_variant_value(db: &NominalDb, enum_ty: TypeAtom, variant: TypeAtom) -> Option<i64> {
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

impl<'a> IrWordGen<'a> {
    fn enum_base_ty(&self, enum_ty: TypeAtom) -> Option<TypeAtom> {
        for e in self.nominals.enums.iter() {
            if e.name != enum_ty {
                continue;
            }
            return Some(e.base);
        }
        None
    }

    fn prim_bits_signed(&self, ty: TypeAtom) -> Option<(u16, bool)> {
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

    fn ty_bits_signed(&self, ty: TypeAtom) -> Option<(u16, bool)> {
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

    fn check_raw_cast_allowed(&self, from_ty: TypeAtom, to_ty: TypeAtom) -> bool {
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

    fn new(
        src: &'a [u8],
        env: &'a [WordEntry],
        subtypes: &'a [SubtypeInfo],
        mmio: &'a MmioDb,
        resources: &'a ResourceDb,
        nominals: &'a NominalDb,
        iso: &'a IsoDb,
        checks: ChecksMode,
        allow_raw_casts: bool,
        sig: &'a WordSig,
        name: lir::Atom,
    ) -> Result<Self, TcError> {
        let mut types: FixedVec<lir::Atom, 64> = FixedVec::new();
        let z = lir::Atom::new(b"").unwrap();
        let _ = types.push(z).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"i64").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"bool").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"str").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"ptr").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"ptr_mut").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        let _ = types.push(lir::Atom::new(b"mmio").unwrap()).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;

        let mut lir_sig = lir::Sig::empty();
        lir_sig.in_len = sig.in_len;
        lir_sig.out_len = sig.out_len;
        for i in 0..(sig.in_len as usize) {
            let atom = lir_atom_lossy(sig.inputs[i].as_bytes());
            lir_sig.inputs[i] = intern_type(&mut types, atom, Span::new(0, 0))?;
        }
        for i in 0..(sig.out_len as usize) {
            let atom = lir_atom_lossy(sig.outputs[i].as_bytes());
            lir_sig.outputs[i] = intern_type(&mut types, atom, Span::new(0, 0))?;
        }

        let mut blocks: FixedVec<lir::Block, 16> = FixedVec::new();
        let mut entry_stack: FixedVec<lir::TypeId, 32> = FixedVec::new();
        for i in 0..(lir_sig.in_len as usize) {
            let _ = entry_stack.push(lir_sig.inputs[i]).map_err(|_| TcError { code: 3902, span: Span::new(0, 0) })?;
        }
        let entry_block = lir::Block { id: lir::BlockId(0), entry_stack, ops: FixedVec::new() };
        let _ = blocks.push(entry_block).map_err(|_| TcError { code: 3903, span: Span::new(0, 0) })?;

        Ok(Self {
            src,
            env,
            subtypes,
            mmio,
            resources,
            nominals,
            iso,
            checks,
            allow_raw_casts,
            sig,
            locals: [TypeAtom::new(b"").unwrap(); 64],
            local_tys: [TypeAtom::new(b"").unwrap(); 64],
            local_live: [false; 64],
            local_scoped: [0u16; 64],
            local_len: 0,
            next_scope: 1,
            scope_stack: [0u16; 16],
            scope_sp: 0,
            locked_resource: None,
            terminated: false,
            word: lir::Word {
                name,
                sig: lir_sig,
                entry: lir::BlockId(0),
                types,
                blocks,
            },
        })
    }

    fn block_mut(&mut self, id: lir::BlockId) -> Result<&mut lir::Block, TcError> {
        self.word
            .blocks
            .get_mut(id.0 as usize)
            .ok_or(TcError { code: 3904, span: Span::new(0, 0) })
    }

    fn new_block(&mut self, stack: &[Value; 256], sp: usize, span: Span) -> Result<lir::BlockId, TcError> {
        let id = lir::BlockId(self.word.blocks.len() as u16);
        let mut entry_stack: FixedVec<lir::TypeId, 32> = FixedVec::new();
        for i in 0..sp {
            let tid = self.ty_id_of_value(stack[i], span)?;
            let _ = entry_stack.push(tid).map_err(|_| TcError { code: 3902, span })?;
        }
        let b = lir::Block {
            id,
            entry_stack,
            ops: FixedVec::new(),
        };
        let _ = self.word.blocks.push(b).map_err(|_| TcError { code: 3903, span })?;
        Ok(id)
    }

    fn emit_op(&mut self, cur: lir::BlockId, kind: lir::OpKind, span: Span) -> Result<(), TcError> {
        let op = lir::Op { kind, span };
        let b = self.block_mut(cur)?;
        let _ = b.ops.push(op).map_err(|_| TcError { code: 3905, span })?;
        Ok(())
    }

    fn check_no_scoped_live(&self, stack: &[Value; 256], sp: usize) -> bool {
        check_no_scoped_live(stack, sp)
    }

    fn any_scoped_live(&self, stack: &[Value; 256], sp: usize) -> bool {
        if !check_no_scoped_live(stack, sp) {
            return true;
        }
        for i in 0..self.local_len {
            if self.local_live[i] && self.local_scoped[i] != 0 {
                return true;
            }
        }
        false
    }

    fn check_no_scoped_live_all(&self, stack: &[Value; 256], sp: usize) -> bool {
        !self.any_scoped_live(stack, sp)
    }

    fn enter_scope(&mut self) -> Option<u16> {
        if self.scope_sp >= self.scope_stack.len() {
            return None;
        }
        let id = self.next_scope;
        self.next_scope = self.next_scope.wrapping_add(1);
        self.scope_stack[self.scope_sp] = id;
        self.scope_sp += 1;
        Some(id)
    }

    fn leave_scope(&mut self, id: u16) {
        if self.scope_sp == 0 {
            return;
        }
        let top = self.scope_stack[self.scope_sp - 1];
        if top == id {
            self.scope_sp -= 1;
        }
    }

    fn stack_has_scope(&self, stack: &[Value; 256], sp: usize, scope: u16) -> bool {
        for i in 0..sp {
            if let Value::Scoped { scope: s, .. } = stack[i] {
                if s == scope {
                    return true;
                }
            }
        }
        false
    }

    fn invalidate_scope_locals(&mut self, scope: u16) {
        for i in 0..self.local_len {
            if self.local_scoped[i] == scope {
                self.local_scoped[i] = 0;
                self.local_live[i] = false;
            }
        }
    }

    fn local_slot(&self, idx: usize) -> u16 {
        self.sig.in_len as u16 + idx as u16
    }

    fn temp_base_slot(&self) -> u16 {
        (self.sig.in_len as u16)
            .wrapping_add(self.local_len as u16)
            .wrapping_add(1)
    }

    fn emit_subtype_range_trap(
        &mut self,
        cur: lir::BlockId,
        value_slot: u16,
        value_ty: lir::TypeId,
        st: &SubtypeInfo,
        span: Span,
    ) -> Result<(), TcError> {
        self.emit_op(cur, lir::OpKind::LocalGet { slot: value_slot, ty: value_ty }, span)?;
        self.emit_op(cur, lir::OpKind::ConstI64(st.min), span)?;
        self.emit_op(
            cur,
            lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Ge,
            },
            span,
        )?;
        self.emit_op(
            cur,
            lir::OpKind::TrapIfFalse {
                code: lir::TrapCode::SubtypeFail,
            },
            span,
        )?;

        self.emit_op(cur, lir::OpKind::LocalGet { slot: value_slot, ty: value_ty }, span)?;
        self.emit_op(cur, lir::OpKind::ConstI64(st.max), span)?;
        self.emit_op(
            cur,
            lir::OpKind::Cmp {
                out: lir::TY_BOOL,
                kind: lir::CmpKind::Le,
            },
            span,
        )?;
        self.emit_op(
            cur,
            lir::OpKind::TrapIfFalse {
                code: lir::TrapCode::SubtypeFail,
            },
            span,
        )?;
        Ok(())
    }

    fn ty_id_of_type(&mut self, ty: TypeAtom, span: Span) -> Result<lir::TypeId, TcError> {
        intern_type(&mut self.word.types, lir_atom_lossy(ty.as_bytes()), span)
    }

    fn ty_id_of_value(&mut self, v: Value, span: Span) -> Result<lir::TypeId, TcError> {
        match v {
            Value::Plain(t) => self.ty_id_of_type(t, span),
            Value::Scoped { ty, .. } => self.ty_id_of_type(ty, span),
            Value::Resource(_) => intern_type(&mut self.word.types, lir_atom_lossy(b"resource"), span),
            Value::Quot(_) => intern_type(&mut self.word.types, lir_atom_lossy(b"quot"), span),
            Value::MmioPlace(_) => Ok(lir::TY_MMIO),
            Value::Ptr { mutable: false, .. } => Ok(lir::TY_PTR),
            Value::Ptr { mutable: true, .. } => Ok(lir::TY_PTR_MUT),
            Value::MmioPtr { mutable: false, .. } => Ok(lir::TY_PTR),
            Value::MmioPtr { mutable: true, .. } => Ok(lir::TY_PTR_MUT),
        }
    }

    fn resolve_place_pointee_ty(&self, place_bytes: &[u8], place_abs: Span) -> Result<Option<TypeAtom>, TcError> {
        // Split `a.b.c` into atoms.
        let mut segs: FixedVec<TypeAtom, 8> = FixedVec::new();
        let mut start = 0usize;
        for i in 0..=place_bytes.len() {
            if i == place_bytes.len() || place_bytes[i] == b'.' {
                if i == start {
                    return Err(TcError { code: 3715, span: place_abs });
                }
                let atom = TypeAtom::new(&place_bytes[start..i]).ok_or(TcError { code: 3715, span: place_abs })?;
                segs.push(atom).map_err(|_| TcError { code: 3715, span: place_abs })?;
                start = i + 1;
            }
        }
        if segs.len() == 0 {
            return Ok(None);
        }

        let root = *segs.get(0).unwrap();
        let mut ty = if let Some(rty) = resource_ty(self.resources, root) {
            rty
        } else if let Some(idx) = find_local(&self.locals, self.local_len, root) {
            self.local_tys[idx]
        } else {
            return Ok(None);
        };

        for i in 1..segs.len() {
            let field = *segs.get(i).unwrap();
            let Some(next) = struct_field_ty(self.nominals, ty, field) else {
                return Err(TcError { code: 3716, span: place_abs });
            };
            ty = next;
        }

        Ok(Some(ty))
    }

    fn lir_sig_for_entry(&mut self, sig: &WordSig, span: Span) -> Result<lir::Sig, TcError> {
        let mut out = lir::Sig::empty();
        out.in_len = sig.in_len;
        out.out_len = sig.out_len;
        for i in 0..(sig.in_len as usize) {
            out.inputs[i] = self.ty_id_of_type(sig.inputs[i], span)?;
        }
        for i in 0..(sig.out_len as usize) {
            out.outputs[i] = self.ty_id_of_type(sig.outputs[i], span)?;
        }
        Ok(out)
    }

    fn emit_prologue(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        requires: Option<Span>,
    ) -> Result<lir::BlockId, TcError> {
        let mut cur = cur;
        let n = self.sig.in_len as usize;
        // Move params into implicit slots 0..n, so checks can access them without disturbing stack order.
        for i in (0..n).rev() {
            let v = pop(stack, sp).ok_or(TcError { code: 3202, span: Span::new(0, 0) })?;
            let _ = v;
            self.emit_op(cur, lir::OpKind::LocalSet { slot: i as u16, ty: self.word.sig.inputs[i] }, Span::new(0, 0))?;
        }

        if self.checks == ChecksMode::All {
            for i in 0..n {
                if let Some(st) = find_subtype(self.subtypes, self.sig.inputs[i]) {
                    self.emit_subtype_range_trap(cur, i as u16, self.word.sig.inputs[i], &st, Span::new(0, 0))?;
                }
            }
        }

        let mut params_on_stack = false;
        if self.checks != ChecksMode::Off && (self.checks == ChecksMode::Contracts || self.checks == ChecksMode::All) {
            if let Some(req) = requires {
                // Reload params onto stack for predicate evaluation.
                for i in 0..n {
                    self.emit_op(cur, lir::OpKind::LocalGet { slot: i as u16, ty: self.word.sig.inputs[i] }, req)?;
                    push(stack, sp, Value::Plain(self.sig.inputs[i]))?;
                }
                cur = self.compile_quote_span(cur, stack, sp, req, false, false)?;
                // Predicate must leave an extra bool, preserving inputs.
                if *sp != n + 1 {
                    return Err(TcError { code: 3310, span: req });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
                    return Err(TcError { code: 3311, span: req });
                }
                for i in 0..n {
                    if stack[i] != Value::Plain(self.sig.inputs[i]) {
                        return Err(TcError { code: 3312, span: req });
                    }
                }
                // Consume bool and keep inputs.
                let _ = pop(stack, sp);
                self.emit_op(cur, lir::OpKind::TrapIfFalse { code: lir::TrapCode::ContractFail }, req)?;
                params_on_stack = true;
            }
        }

        // Reload params for word body execution.
        if !params_on_stack {
            for i in 0..n {
                self.emit_op(cur, lir::OpKind::LocalGet { slot: i as u16, ty: self.word.sig.inputs[i] }, Span::new(0, 0))?;
                push(stack, sp, Value::Plain(self.sig.inputs[i]))?;
            }
        }
        Ok(cur)
    }

    fn emit_epilogue(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        ensures: Option<Span>,
    ) -> Result<lir::BlockId, TcError> {
        let mut cur = cur;

        if self.checks != ChecksMode::Off && (self.checks == ChecksMode::Contracts || self.checks == ChecksMode::All) {
            if let Some(ens) = ensures {
                let n = self.sig.out_len as usize;
                let base_sp = *sp;
                // Evaluate predicate with outputs on stack.
                cur = self.compile_quote_span(cur, stack, sp, ens, false, false)?;
                if *sp != base_sp + 1 {
                    return Err(TcError { code: 3320, span: ens });
                }
                if stack[*sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
                    return Err(TcError { code: 3321, span: ens });
                }
                for i in 0..n {
                    if stack[i] != Value::Plain(self.sig.outputs[i]) {
                        return Err(TcError { code: 3322, span: ens });
                    }
                }
                let _ = pop(stack, sp);
                self.emit_op(cur, lir::OpKind::TrapIfFalse { code: lir::TrapCode::ContractFail }, ens)?;
            }
        }

        if self.checks == ChecksMode::All {
            // Check subtype returns without disturbing order.
            let n = self.sig.out_len as usize;
            let base_stack = *stack;
            let base_sp = *sp;
            let tmp_base = self.temp_base_slot();
            for i in (0..n).rev() {
                let v = pop(stack, sp).ok_or(TcError { code: 3202, span: Span::new(0, 0) })?;
                let _ = v;
                self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp_base + i as u16, ty: self.word.sig.outputs[i] }, Span::new(0, 0))?;
            }
            for i in 0..n {
                if let Some(st) = find_subtype(self.subtypes, self.sig.outputs[i]) {
                    self.emit_subtype_range_trap(cur, tmp_base + i as u16, self.word.sig.outputs[i], &st, Span::new(0, 0))?;
                }
            }
            for i in 0..n {
                self.emit_op(
                    cur,
                    lir::OpKind::LocalGet { slot: tmp_base + i as u16, ty: self.word.sig.outputs[i] },
                    Span::new(0, 0),
                )?;
                stack[i] = base_stack[i];
            }
            *sp = base_sp;
        }

        Ok(cur)
    }

    // (emit_local_set / emit_op_checked removed in v1.1 IR path)

    fn compile_quote_span(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        quot_span: Span,
        allow_suspend: bool,
        allow_locals: bool,
    ) -> Result<lir::BlockId, TcError> {
        // Expect brackets at ends; just slice inside.
        if quot_span.end <= quot_span.start + 2 {
            return Ok(cur);
        }
        let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
        self.compile_span(cur, stack, sp, inner, allow_suspend, allow_locals)
    }

    fn compile_span(
        &mut self,
        mut cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
        allow_suspend: bool,
        allow_locals: bool,
    ) -> Result<lir::BlockId, TcError> {
        let slice = &self.src[span.start..span.end];
        let mut lex = Lexer::new(slice);
        let mut terminated = false;

        loop {
            let tok = lex.next();
            if tok.kind == TokenKind::Eof {
                break;
            }
            if terminated {
                continue;
            }

            match tok.kind {
                TokenKind::Number => {
                    push(stack, sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?;
                    let num = parse_i64_token(&slice[tok.span.start..tok.span.end]).unwrap_or(0);
                    self.emit_op(cur, lir::OpKind::ConstI64(num), Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                }
                TokenKind::String => {
                    push(stack, sp, Value::Plain(TypeAtom::new(b"str").unwrap()))?;
                    self.emit_op(cur, lir::OpKind::ConstStr(Span::new(span.start + tok.span.start, span.start + tok.span.end)), Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                }
                TokenKind::PunctArrowBind => {
                    if !allow_locals {
                        return Err(TcError { code: 3281, span });
                    }
                    let name = lex.next();
                    if name.kind != TokenKind::Ident {
                        return Err(TcError { code: 3201, span: Span::new(span.start + name.span.start, span.start + name.span.end) });
                    }
                    let v = pop(stack, sp).ok_or(TcError { code: 3202, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
                    if v == Value::Plain(TypeAtom::new(b"scoped").unwrap()) {
                        return Err(TcError { code: 3504, span });
                    }
	                    let ty = match v {
	                        Value::Plain(t) => t,
	                        Value::Scoped { ty, .. } => ty,
	                        Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                        Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                        Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                        Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                        Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                        Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                    };
                    let lname = TypeAtom::new(&slice[name.span.start..name.span.end]).ok_or(TcError {
                        code: 3203,
                        span: Span::new(span.start + name.span.start, span.start + name.span.end),
                    })?;
                    if find_local(&self.locals, self.local_len, lname).is_some() {
                        return Err(TcError { code: 3204, span: Span::new(span.start + name.span.start, span.start + name.span.end) });
                    }
                    if self.local_len >= self.locals.len() {
                        return Err(TcError { code: 3205, span });
                    }
                    let slot = self.local_slot(self.local_len);
                    self.locals[self.local_len] = lname;
                    self.local_tys[self.local_len] = ty;
                    self.local_live[self.local_len] = true;
                    self.local_scoped[self.local_len] = match v {
                        Value::Scoped { scope, .. } => scope,
                        _ => 0u16,
                    };
                    self.local_len += 1;
                    let tid = self.ty_id_of_type(ty, span)?;
                    self.emit_op(cur, lir::OpKind::LocalSet { slot, ty: tid }, Span::new(span.start + tok.span.start, span.start + tok.span.end))?;
                }
                TokenKind::PunctLBracket => {
                    let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                        .map_err(|code| TcError { code, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
                    let q_span = Span::new(span.start + q.start, span.start + q.end);
                    push(stack, sp, Value::Quot(q_span))?;
                }
	                TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
	                    let mut_tok = tok.kind == TokenKind::PunctAmpBang;
	                    let place = parse_place(&mut lex, slice).ok_or(TcError { code: 3500, span: Span::new(span.start + tok.span.start, span.start + tok.span.end) })?;
	                    let place_bytes = &slice[place.full.start..place.full.end];
	                    let place_abs = Span::new(span.start + place.full.start, span.start + place.full.end);
	                    let root_atom = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap_or(TypeAtom::new(b"").unwrap());
	                    let mut const_addr: Option<u64> = None;

	                    if let Some(res) = resolve_mmio_place(self.mmio, self.src, place_bytes, place_abs)? {
	                        match res {
	                            MmioResolved::Reg(reg) => {
	                                if mut_tok && !access_can_write(reg.access) {
	                                    return Err(TcError { code: 3609, span: place_abs });
	                                }
	                                const_addr = Some(reg.addr);
	                                push(stack, sp, Value::MmioPtr { reg, mutable: mut_tok })?;
	                            }
	                            MmioResolved::Field(_) => return Err(TcError { code: 3608, span: place_abs }),
	                        }
	                    } else {
                        if resource_ty(self.resources, root_atom).is_some() {
                            // Resources are only borrowable inside their own lock scope.
                            if self.locked_resource != Some(root_atom) {
                                return Err(TcError { code: 3515, span: place_abs });
                            }
                        } else if mut_tok {
                            let root = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap();
                            if find_local(&self.locals, self.local_len, root).is_some() {
                                return Err(TcError { code: 3501, span: place.root_abs(span.start) });
                            }
                        }
                        if let Some(pointee) = self.resolve_place_pointee_ty(place_bytes, place_abs)? {
                            push(stack, sp, Value::Ptr { ty: pointee, mutable: mut_tok })?;
                        } else {
                            let ty = if mut_tok { TypeAtom::new(b"ptr_mut").unwrap() } else { TypeAtom::new(b"ptr").unwrap() };
                            push(stack, sp, Value::Plain(ty))?;
                        }
                    }

	                    self.emit_op(
	                        cur,
	                        lir::OpKind::AddrOf {
	                            place: lir_atom_lossy(place_bytes),
	                            mutable: mut_tok,
	                            const_addr,
	                        },
	                        place_abs,
	                    )?;
	                }
                TokenKind::PunctPipeGreater => {
                    // `|>` send: ( Chan(T) T -- )
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let val = pop(stack, sp).ok_or(TcError { code: 3730, span: op_span })?;
                    let ch = pop(stack, sp).ok_or(TcError { code: 3730, span: op_span })?;

                    let val_ty = match val {
                        Value::Plain(t) => t,
                        Value::Scoped { ty, .. } => ty,
                        Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
                        Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                        Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
                        Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                        Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                        Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                        Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                    };
                    let ch_ty = match ch {
                        Value::Plain(t) => t,
                        _ => return Err(TcError { code: 3731, span: op_span }),
                    };
                    let elem = chan_elem_type(ch_ty).ok_or(TcError { code: 3731, span: op_span })?;
                    if !type_compatible(val_ty, elem, self.subtypes) {
                        return Err(TcError { code: 3732, span: op_span });
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
                            name: lir_atom_lossy(b"platform.channel.send"),
                            sig,
                            may_suspend: false,
                        },
                        op_span,
                    )?;
                }
                TokenKind::PunctLessPipe => {
                    // `<|` receive: ( Chan(T) -- T )
                    let op_span = Span::new(span.start + tok.span.start, span.start + tok.span.end);
                    let ch = pop(stack, sp).ok_or(TcError { code: 3733, span: op_span })?;
                    let ch_ty = match ch {
                        Value::Plain(t) => t,
                        _ => return Err(TcError { code: 3734, span: op_span }),
                    };
                    let elem = chan_elem_type(ch_ty).ok_or(TcError { code: 3734, span: op_span })?;
                    push(stack, sp, Value::Plain(elem))?;

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
                            name: lir_atom_lossy(b"platform.channel.recv"),
                            sig,
                            may_suspend: false,
                        },
                        op_span,
                    )?;
                }
                TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                    let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
                    if *sp == 0 {
                        return Err(TcError { code: 3505, span });
                    }
                    let top = stack[*sp - 1];
	                    let top_ty = match top {
	                        Value::Plain(t) => t,
	                        Value::Scoped { ty, .. } => ty,
	                        Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                        Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                        Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                        Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                        Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                        Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                    };

                    if let Some(elem) = array_elem_type(top_ty) {
                        // Milestone 13: Arrays -> scoped slices.
                        let scope_id = self.enter_scope().ok_or(TcError { code: 3512, span })?;
                        let slice_ty = slice_type_of_elem(elem, mut_scope).ok_or(TcError { code: 3513, span })?;
                        push(
                            stack,
                            sp,
                            Value::Scoped {
                                ty: slice_ty,
                                scope: scope_id,
                            },
                        )?;
                        let tid = self.ty_id_of_type(slice_ty, span)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::ScopedEnter { ty: tid },
                            Span::new(span.start + tok.span.start, span.start + tok.span.end),
                        )?;

                        let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span })?;
                        let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                        cur = self.compile_span(
                            cur,
                            stack,
                            sp,
                            Span::new(span.start + block.inner_start, span.start + block.inner_end),
                            block_allow_suspend,
                            allow_locals,
                        )?;

                        if self.stack_has_scope(stack, *sp, scope_id) {
                            return Err(TcError { code: 3506, span });
                        }
                        self.invalidate_scope_locals(scope_id);
                        self.leave_scope(scope_id);
                    } else if top_ty == TypeAtom::new(b"Region").unwrap() {
                        // Milestone 5: Regions -> scoped borrowed region handles.
                        let scope_id = self.enter_scope().ok_or(TcError { code: 3512, span })?;
                        let rty = region_ref_type(mut_scope);
                        push(
                            stack,
                            sp,
                            Value::Scoped {
                                ty: rty,
                                scope: scope_id,
                            },
                        )?;
                        let tid = self.ty_id_of_type(rty, span)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::ScopedEnter { ty: tid },
                            Span::new(span.start + tok.span.start, span.start + tok.span.end),
                        )?;

                        let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span })?;
                        let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                        cur = self.compile_span(
                            cur,
                            stack,
                            sp,
                            Span::new(span.start + block.inner_start, span.start + block.inner_end),
                            block_allow_suspend,
                            allow_locals,
                        )?;

                        if self.stack_has_scope(stack, *sp, scope_id) {
                            return Err(TcError { code: 3506, span });
                        }
                        self.invalidate_scope_locals(scope_id);
                        self.leave_scope(scope_id);
                    } else {
                        // `&[` / `&![` only apply to Arrays and Regions in v1.
                        return Err(TcError { code: 3515, span });
                    }
                }
                TokenKind::Ident | TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                    let mut qbuf = [0u8; 64];
                    let (name, name_span) = if tok.kind == TokenKind::Ident {
                        let (len, used, s) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                        let bytes = if used { &qbuf[..len] } else { &slice[tok.span.start..tok.span.end] };
                        (bytes, s)
                    } else {
                        (&slice[tok.span.start..tok.span.end], tok.span)
                    };
                    let name_abs = Span::new(span.start + name_span.start, span.start + name_span.end);

                    if name == b"true" || name == b"false" {
                        push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                        self.emit_op(cur, lir::OpKind::ConstBool(name == b"true"), name_abs)?;
                        continue;
                    }

                    if tok.kind == TokenKind::Ident {
                        if let Some(atom) = TypeAtom::new(name) {
                            if resource_ty(self.resources, atom).is_some() {
                                push(stack, sp, Value::Resource(atom))?;
                                continue;
                            }
                        }
                    }

                    if tok.kind == TokenKind::Ident {
                        // Enum tag literal: `Enum.Variant`
                        if let Some(dot) = name.iter().position(|&b| b == b'.') {
                            if dot + 1 < name.len() && !name[dot + 1..].iter().any(|&b| b == b'.') {
                                let enum_part = &name[..dot];
                                let var_part = &name[dot + 1..];
                                if let (Some(enum_ty), Some(var)) = (TypeAtom::new(enum_part), TypeAtom::new(var_part)) {
                                    if let Some(v) = enum_variant_value(self.nominals, enum_ty, var) {
                                        // Lower as: `ConstI64(v); Cast i64 -> EnumTy`
                                        self.emit_op(cur, lir::OpKind::ConstI64(v), name_abs)?;
                                        push(stack, sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?;
                                        let to = self.ty_id_of_type(enum_ty, name_abs)?;
                                        self.emit_op(cur, lir::OpKind::Cast { from: lir::TY_I64, to }, name_abs)?;
                                        let _ = pop(stack, sp);
                                        push(stack, sp, Value::Plain(enum_ty))?;
                                        continue;
                                    }
                                    // If the enum exists but the variant doesn't, prefer a stable enum diagnostic.
                                    for e in self.nominals.enums.iter() {
                                        if e.name == enum_ty {
                                            return Err(TcError { code: 3725, span: name_abs });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
                        let is_load = name[0] == b'@';
                        let typed = name.len() > 1;
                        let ty_atom = if typed {
                            Some(TypeAtom::new(&name[1..]).ok_or(TcError { code: 3632, span: name_abs })?)
                        } else {
                            None
                        };

                        if is_load {
                            let addr = pop(stack, sp).ok_or(TcError { code: 3633, span: name_abs })?;
                            match addr {
                                Value::MmioPtr { reg, .. } => {
                                    if !access_can_read(reg.access) {
                                        return Err(TcError { code: 3610, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolLoad {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                    if !access_can_read(reg.access) {
                                        return Err(TcError { code: 3610, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    push(stack, sp, Value::Plain(reg.reg_ty))?;
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolLoad {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
	                                Value::MmioPlace(MmioResolved::Field(field)) => {
	                                    if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
	                                        return Err(TcError { code: 3610, span: name_abs });
	                                    }
	                                    if typed {
	                                        return Err(TcError { code: 3634, span: name_abs });
	                                    }
	                                    let (mask, shift) = field_mask_shift(&field.field);
	                                    push(stack, sp, Value::Plain(field.field.ty))?;
	                                    let reg_tid = self.ty_id_of_type(field.reg_ty, name_abs)?;
	                                    let tid = self.ty_id_of_type(field.field.ty, name_abs)?;
	                                    self.emit_op(
	                                        cur,
	                                        lir::OpKind::MmioVolLoadField {
	                                            reg_ty: reg_tid,
	                                            field_ty: tid,
	                                            place: lir_atom_lossy(slice_span(self.src, field.place_span)),
	                                            mask,
	                                            shift,
	                                        },
	                                        name_abs,
	                                    )?;
	                                    continue;
	                                }
                                Value::Ptr { ty, .. } => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let want = ty_atom.unwrap();
                                    if want != ty {
                                        return Err(TcError { code: 3717, span: name_abs });
                                    }
                                    push(stack, sp, Value::Plain(ty))?;
                                    let tid = self.ty_id_of_type(ty, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, name_abs)?;
                                    continue;
                                }
                                Value::Plain(t) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if t != TypeAtom::new(b"ptr").unwrap() && t != TypeAtom::new(b"ptr_mut").unwrap() {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let ty_atom = ty_atom.unwrap();
                                    push(stack, sp, Value::Plain(ty_atom))?;
                                    let tid = self.ty_id_of_type(ty_atom, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Load { ty: tid }, name_abs)?;
                                    continue;
                                }
                                _ => return Err(TcError { code: 3614, span: name_abs }),
                            }
                        } else {
                            let v = pop(stack, sp).ok_or(TcError { code: 3230, span: name_abs })?;
                            let addr = pop(stack, sp).ok_or(TcError { code: 3230, span: name_abs })?;
	                            let vty = match v {
	                                Value::Plain(t) => t,
	                                Value::Scoped { ty, .. } => ty,
	                                Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                                Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                                Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                                Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                                Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            };
                            match (addr, v) {
                                (Value::MmioPtr { reg, mutable: false }, _) => {
                                    let _ = reg;
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                (Value::MmioPtr { reg, mutable: true }, Value::Plain(_)) => {
                                    if !access_can_write(reg.access) {
                                        return Err(TcError { code: 3609, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    if vty != reg.reg_ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolStore {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
                                (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(_)) => {
                                    if !access_can_write(reg.access) {
                                        return Err(TcError { code: 3609, span: name_abs });
                                    }
                                    if let Some(want) = ty_atom {
                                        if want != reg.reg_ty {
                                            return Err(TcError { code: 3613, span: name_abs });
                                        }
                                    }
                                    if vty != reg.reg_ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(reg.reg_ty, name_abs)?;
                                    self.emit_op(
                                        cur,
                                        lir::OpKind::MmioVolStore {
                                            ty: tid,
                                            place: lir_atom_lossy(slice_span(self.src, reg.place_span)),
                                        },
                                        name_abs,
                                    )?;
                                    continue;
                                }
	                                (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(_)) => {
	                                    if !access_can_write(field.reg_access) || !access_can_write(field.field.access) {
	                                        return Err(TcError { code: 3609, span: name_abs });
	                                    }
	                                    if typed {
	                                        return Err(TcError { code: 3634, span: name_abs });
	                                    }
	                                    if vty != field.field.ty {
	                                        return Err(TcError { code: 3231, span: name_abs });
	                                    }
	                                    let (mask, shift) = field_mask_shift(&field.field);
	                                    let reg_tid = self.ty_id_of_type(field.reg_ty, name_abs)?;
	                                    let tid = self.ty_id_of_type(field.field.ty, name_abs)?;
	                                    self.emit_op(
	                                        cur,
	                                        lir::OpKind::MmioVolStoreField {
	                                            reg_ty: reg_tid,
	                                            field_ty: tid,
	                                            place: lir_atom_lossy(slice_span(self.src, field.place_span)),
	                                            mask,
	                                            shift,
	                                        },
	                                        name_abs,
	                                    )?;
	                                    continue;
	                                }
                                (Value::Ptr { ty, mutable }, Value::Plain(_)) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if !mutable {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let want = ty_atom.unwrap();
                                    if want != ty {
                                        return Err(TcError { code: 3717, span: name_abs });
                                    }
                                    if vty != ty {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(ty, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Store { ty: tid }, name_abs)?;
                                    continue;
                                }
                                (Value::Plain(t), Value::Plain(_)) => {
                                    if !typed {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    if t != TypeAtom::new(b"ptr_mut").unwrap() {
                                        return Err(TcError { code: 3614, span: name_abs });
                                    }
                                    let ty_atom = ty_atom.unwrap();
                                    if vty != ty_atom {
                                        return Err(TcError { code: 3231, span: name_abs });
                                    }
                                    let tid = self.ty_id_of_type(ty_atom, name_abs)?;
                                    self.emit_op(cur, lir::OpKind::Store { ty: tid }, name_abs)?;
                                    continue;
                                }
                                _ => return Err(TcError { code: 3614, span: name_abs }),
                            }
                        }
                    }

	                    if tok.kind == TokenKind::Ident {
	                        if let Some(res) = resolve_mmio_place(self.mmio, self.src, name, name_abs)? {
	                            let addr = match res {
	                                MmioResolved::Reg(reg) => reg.addr,
	                                MmioResolved::Field(field) => field.addr,
	                            };
	                            push(stack, sp, Value::MmioPlace(res))?;
	                            self.emit_op(
	                                cur,
	                                lir::OpKind::MmioPlace {
	                                    place: lir_atom_lossy(name),
	                                    addr,
	                                },
	                                name_abs,
	                            )?;
	                            continue;
	                        }
	                    }

	                    if name == b"dup" {
	                        let top = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
	                        let top_ty = match top {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
	                        if is_iso_type(self.iso, top_ty) {
	                            return Err(TcError { code: 3742, span: name_abs });
	                        }
	                        push(stack, sp, top)?;
	                        push(stack, sp, top)?;
	                        let tid = self.ty_id_of_value(top, name_abs)?;
	                        self.emit_op(cur, lir::OpKind::Dup { ty: tid }, name_abs)?;
	                        continue;
	                    }
	                    if name == b"drop" {
	                        let top = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
	                        let top_ty = match top {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
	                        if is_iso_type(self.iso, top_ty) {
	                            return Err(TcError { code: 3743, span: name_abs });
	                        }
	                        let tid = self.ty_id_of_value(top, name_abs)?;
	                        self.emit_op(cur, lir::OpKind::Drop { ty: tid }, name_abs)?;
	                        continue;
	                    }
                    if name == b"swap" {
                        let b = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        let a = pop(stack, sp).ok_or(TcError { code: 3202, span })?;
                        push(stack, sp, b)?;
                        push(stack, sp, a)?;
                        let a_id = self.ty_id_of_value(a, name_abs)?;
                        let b_id = self.ty_id_of_value(b, name_abs)?;
                        self.emit_op(cur, lir::OpKind::Swap { a: a_id, b: b_id }, name_abs)?;
                        continue;
                    }

	                    if name == b"as" || name == b"as?" || name == b"bitcast" {
	                        let first = lex.next();
	                        if first.kind != TokenKind::Ident {
	                            return Err(TcError { code: 3295, span: Span::new(span.start + first.span.start, span.start + first.span.end) });
	                        }
		                        let mut end = first.span.end;
		                        let mut probe = lex;
		                        let lparen = probe.next();
		                        if lparen.kind == TokenKind::PunctLParen {
		                            lex = probe; // consume '('
		                            let par = capture_balanced(&mut lex, slice, TokenKind::PunctLParen, TokenKind::PunctRParen, lparen.span.start)
		                                .map_err(|code| TcError { code, span: Span::new(span.start + lparen.span.start, span.start + lparen.span.end) })?;
		                            end = par.end;
		                        }
	                        let to_span = Span::new(first.span.start, end);
	                        let to_ty = TypeAtom::new(&slice[to_span.start..to_span.end]).ok_or(TcError {
	                            code: 3296,
	                            span: Span::new(span.start + to_span.start, span.start + to_span.end),
	                        })?;
	                        let v = pop(stack, sp).ok_or(TcError { code: 3297, span })?;
		                        let from_ty = match v {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
	                        let subtype = find_subtype(self.subtypes, to_ty);
	                        if let Some(st) = subtype {
	                            if !type_compatible(from_ty, st.base, self.subtypes) {
	                                return Err(TcError { code: if name == b"as?" { 3298 } else { 3300 }, span });
	                            }
	                        }

	                        if name == b"bitcast" {
	                            // Disallow bitcasting directly to/from subtypes (use `as`/`as?` + checks instead).
	                            if subtype.is_some() {
	                                return Err(TcError { code: 3302, span: name_abs });
	                            }
	                            let Some((from_bits, _)) = self.ty_bits_signed(from_ty) else {
	                                return Err(TcError { code: 3303, span: name_abs });
	                            };
	                            let Some((to_bits, _)) = self.ty_bits_signed(to_ty) else {
	                                return Err(TcError { code: 3303, span: name_abs });
	                            };
	                            if from_bits != to_bits {
	                                return Err(TcError { code: 3304, span: name_abs });
	                            }
	                            if !self.check_raw_cast_allowed(from_ty, to_ty) {
	                                return Err(TcError { code: 3305, span: name_abs });
	                            }
	                        } else {
	                            if !self.check_raw_cast_allowed(from_ty, to_ty) {
	                                return Err(TcError { code: 3305, span: name_abs });
	                            }
	                        }

	                        // Model `as`/`as?` as an explicit type cast + optional subtype check.
	                        let from_id = self.ty_id_of_type(from_ty, name_abs)?;
	                        let to_id = self.ty_id_of_type(to_ty, name_abs)?;
	                        if name == b"bitcast" {
	                            self.emit_op(cur, lir::OpKind::Bitcast { from: from_id, to: to_id }, name_abs)?;
	                        } else {
	                            self.emit_op(cur, lir::OpKind::Cast { from: from_id, to: to_id }, name_abs)?;
	                        }
                        if name == b"as?" {
                            push(stack, sp, Value::Plain(to_ty))?;
                            if let Some(st) = subtype {
                                let tmp = self.temp_base_slot();
                                self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: to_id }, name_abs)?;

                                // Stack effect: `( to -- to ok )`
                                self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::ConstI64(st.min), name_abs)?;
                                self.emit_op(
                                    cur,
                                    lir::OpKind::Cmp {
                                        out: lir::TY_BOOL,
                                        kind: lir::CmpKind::Ge,
                                    },
                                    name_abs,
                                )?;
                                self.emit_op(cur, lir::OpKind::LocalGet { slot: tmp, ty: to_id }, name_abs)?;
                                self.emit_op(cur, lir::OpKind::ConstI64(st.max), name_abs)?;
                                self.emit_op(
                                    cur,
                                    lir::OpKind::Cmp {
                                        out: lir::TY_BOOL,
                                        kind: lir::CmpKind::Le,
                                    },
                                    name_abs,
                                )?;
                                self.emit_op(cur, lir::OpKind::AndBool, name_abs)?;
                                push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                            } else {
                                self.emit_op(cur, lir::OpKind::ConstBool(true), name_abs)?;
                                push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                            }
                            continue;
                        }
                        // `as` / `bitcast`
                        push(stack, sp, Value::Plain(to_ty))?;
                        if name == b"as" && subtype.is_some() && self.checks == ChecksMode::All {
                            let tmp = self.temp_base_slot();
                            let st = subtype.unwrap();
                            self.emit_op(cur, lir::OpKind::Dup { ty: to_id }, name_abs)?;
                            self.emit_op(cur, lir::OpKind::LocalSet { slot: tmp, ty: to_id }, name_abs)?;
                            self.emit_subtype_range_trap(cur, tmp, to_id, &st, name_abs)?;
                        }
                        continue;
                    }

                    if name == b"if" {
                        cur = self.compile_if(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"while" {
                        cur = self.compile_while(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"loop" {
                        cur = self.compile_loop(cur, stack, sp, allow_suspend, name_abs)?;
                        continue;
                    }
                    if name == b"return" {
                        let want = self.sig.out_len as usize;
                        if *sp != want {
                            return Err(TcError { code: 3230, span });
                        }
                        if self.any_scoped_live(stack, *sp) {
                            return Err(TcError { code: 3511, span });
                        }
	                        for i in 0..want {
	                            let got = match stack[i] {
	                                Value::Plain(t) => t,
	                                Value::Scoped { ty, .. } => ty,
	                                Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                                Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                                Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                                Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                                Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                                Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            };
                            if !type_compatible(got, self.sig.outputs[i], self.subtypes) {
                                return Err(TcError { code: 3231, span });
                            }
                        }
                        self.emit_op(cur, lir::OpKind::Ret, name_abs)?;
                        terminated = true;
                        self.terminated = true;
                        continue;
                    }
                    if name == b"lock" {
                        cur = self.compile_lock(cur, stack, sp, name_abs)?;
                        continue;
                    }
                    if name == b"platform.task.run" {
                        if !self.check_no_scoped_live_all(stack, *sp) {
                            return Err(TcError { code: 3502, span: name_abs });
                        }
                        let body_q = pop(stack, sp).ok_or(TcError { code: 3750, span: name_abs })?;
                        let body_span = match body_q {
                            Value::Quot(s) => s,
                            _ => return Err(TcError { code: 3751, span: name_abs }),
                        };
                        let base_stack = *stack;
                        let base_sp = *sp;
                        cur = self.compile_quote_span(cur, stack, sp, body_span, true, false)?;
                        if *sp != base_sp {
                            return Err(TcError { code: 3752, span: name_abs });
                        }
                        for i in 0..base_sp {
                            if stack[i] != base_stack[i] {
                                return Err(TcError { code: 3753, span: name_abs });
                            }
                        }
                        continue;
                    }

                    if let Some(idx) = find_local(&self.locals, self.local_len, TypeAtom::new(name).unwrap_or(TypeAtom::new(b"").unwrap())) {
                        if !self.local_live[idx] {
                            return Err(TcError { code: 3514, span: name_abs });
                        }
	                        if is_iso_type(self.iso, self.local_tys[idx]) {
	                            self.local_live[idx] = false;
	                        }
	                        if self.local_scoped[idx] != 0 {
	                            push(
	                                stack,
	                                sp,
                                Value::Scoped {
                                    ty: self.local_tys[idx],
                                    scope: self.local_scoped[idx],
                                },
                            )?;
                        } else {
                            push(stack, sp, Value::Plain(self.local_tys[idx]))?;
                        }
                        let tid = self.ty_id_of_type(self.local_tys[idx], name_abs)?;
                        self.emit_op(cur, lir::OpKind::LocalGet { slot: self.local_slot(idx), ty: tid }, name_abs)?;
                        continue;
                    }

                    let entry = lookup(self.env, name).ok_or(TcError { code: 3210, span: name_abs })?;
                    if entry.may_suspend && !allow_suspend {
                        return Err(TcError { code: 3503, span: name_abs });
                    }
                    if entry.may_suspend && !self.check_no_scoped_live_all(stack, *sp) {
                        return Err(TcError { code: 3502, span: name_abs });
                    }
                    apply_sig(stack, sp, entry, name_abs, self.subtypes)?;

                    let builtin = match name {
                        b"+" => Some(lir::OpKind::AddI64),
                        b"-" => Some(lir::OpKind::SubI64),
                        b"*" => Some(lir::OpKind::MulI64),
                        b">" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Gt }),
                        b"<" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Lt }),
                        b">=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Ge }),
                        b"<=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Le }),
                        b"==" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Eq }),
                        b"!=" => Some(lir::OpKind::Cmp { out: lir::TY_BOOL, kind: lir::CmpKind::Ne }),
                        b"and" => Some(lir::OpKind::AndBool),
                        b"or" => Some(lir::OpKind::OrBool),
                        b"not" => Some(lir::OpKind::NotBool),
                        _ => None,
                    };
                    if let Some(kind) = builtin {
                        self.emit_op(cur, kind, name_abs)?;
                    } else {
                        let call_sig = self.lir_sig_for_entry(&entry.sig, name_abs)?;
                        self.emit_op(
                            cur,
                            lir::OpKind::Call {
                                name: lir_atom_lossy(name),
                                sig: call_sig,
                                may_suspend: entry.may_suspend,
                            },
                            name_abs,
                        )?;
                    }
                }
                _ => {
                    // ignore other punctuation in MVP
                }
            }
        }

        Ok(cur)
    }

    fn compile_if(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        let else_q = pop(stack, sp).ok_or(TcError { code: 3240, span })?;
        let then_q = pop(stack, sp).ok_or(TcError { code: 3241, span })?;
        let cond = pop(stack, sp).ok_or(TcError { code: 3242, span })?;
        if cond != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
            return Err(TcError { code: 3243, span });
        }
        let then_span = match then_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3244, span }),
        };
        let else_span = match else_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3245, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let then_blk = self.new_block(&base_stack, base_sp, span)?;
        let else_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::BrIf { then_tgt: then_blk, else_tgt: else_blk }, span)?;

        let mut then_stack = base_stack;
        let mut then_sp = base_sp;
        let then_end = self.compile_quote_span(then_blk, &mut then_stack, &mut then_sp, then_span, allow_suspend, false)?;

        let mut else_stack = base_stack;
        let mut else_sp = base_sp;
        let else_end = self.compile_quote_span(else_blk, &mut else_stack, &mut else_sp, else_span, allow_suspend, false)?;

        if then_sp != else_sp {
            return Err(TcError { code: 3246, span });
        }
        for i in 0..then_sp {
            if then_stack[i] != else_stack[i] {
                return Err(TcError { code: 3247, span });
            }
        }

        let join_blk = self.new_block(&then_stack, then_sp, span)?;
        self.emit_op(then_end, lir::OpKind::Br { target: join_blk }, span)?;
        self.emit_op(else_end, lir::OpKind::Br { target: join_blk }, span)?;

        for i in 0..then_sp {
            stack[i] = then_stack[i];
        }
        *sp = then_sp;
        Ok(join_blk)
    }

    fn compile_while(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError { code: 3250, span })?;
        let cond_q = pop(stack, sp).ok_or(TcError { code: 3251, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3252, span }),
        };
        let cond_span = match cond_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3253, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let header = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::Br { target: header }, span)?;

        let mut cond_stack = base_stack;
        let mut cond_sp = base_sp;
        let cond_end = self.compile_quote_span(header, &mut cond_stack, &mut cond_sp, cond_span, allow_suspend, false)?;
        if cond_sp != base_sp + 1 {
            return Err(TcError { code: 3254, span });
        }
        if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
            return Err(TcError { code: 3255, span });
        }
        for i in 0..base_sp {
            if cond_stack[i] != base_stack[i] {
                return Err(TcError { code: 3256, span });
            }
        }
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cond_end, lir::OpKind::BrIf { then_tgt: body_blk, else_tgt: after_blk }, span)?;

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        let body_end = self.compile_quote_span(body_blk, &mut body_stack, &mut body_sp, body_span, allow_suspend, false)?;
        if body_sp != base_sp {
            return Err(TcError { code: 3257, span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError { code: 3258, span });
            }
        }
        self.emit_op(body_end, lir::OpKind::Br { target: header }, span)?;

        *stack = base_stack;
        *sp = base_sp;
        Ok(after_blk)
    }

    fn compile_loop(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        allow_suspend: bool,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        let body_q = pop(stack, sp).ok_or(TcError { code: 3260, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3261, span }),
        };

        let base_stack = *stack;
        let base_sp = *sp;

        let check_blk = self.new_block(&base_stack, base_sp, span)?;
        self.emit_op(cur, lir::OpKind::Br { target: check_blk }, span)?;
        let body_blk = self.new_block(&base_stack, base_sp, span)?;
        let after_blk = self.new_block(&base_stack, base_sp, span)?;

        self.emit_op(check_blk, lir::OpKind::ConstBool(true), span)?;
        self.emit_op(check_blk, lir::OpKind::BrIf { then_tgt: body_blk, else_tgt: after_blk }, span)?;

        let mut body_stack = base_stack;
        let mut body_sp = base_sp;
        let body_end = self.compile_quote_span(body_blk, &mut body_stack, &mut body_sp, body_span, allow_suspend, false)?;
        if body_sp != base_sp {
            return Err(TcError { code: 3262, span });
        }
        for i in 0..base_sp {
            if body_stack[i] != base_stack[i] {
                return Err(TcError { code: 3263, span });
            }
        }
        self.emit_op(body_end, lir::OpKind::Br { target: check_blk }, span)?;

        *stack = base_stack;
        *sp = base_sp;
        Ok(after_blk)
    }

    fn compile_lock(
        &mut self,
        cur: lir::BlockId,
        stack: &mut [Value; 256],
        sp: &mut usize,
        span: Span,
    ) -> Result<lir::BlockId, TcError> {
        if self.locked_resource.is_some() {
            return Err(TcError { code: 3517, span });
        }
        let body_q = pop(stack, sp).ok_or(TcError { code: 3270, span })?;
        let body_span = match body_q {
            Value::Quot(s) => s,
            _ => return Err(TcError { code: 3271, span }),
        };
        let locked = if *sp > 0 {
            match stack[*sp - 1] {
                Value::Resource(name) => {
                    let _ = pop(stack, sp);
                    Some(name)
                }
                _ => None,
            }
        } else {
            None
        };
        self.locked_resource = locked;
        let base_stack = *stack;
        let base_sp = *sp;
        let end = self.compile_quote_span(cur, stack, sp, body_span, false, false)?;
        if *sp != base_sp {
            return Err(TcError { code: 3272, span });
        }
        for i in 0..base_sp {
            if stack[i] != base_stack[i] {
                return Err(TcError { code: 3273, span });
            }
        }
        self.locked_resource = None;
        Ok(end)
    }
}

fn parse_i64_token(token: &[u8]) -> Option<i64> {
    if token.is_empty() {
        return None;
    }
    let mut v: i64 = 0;
    let mut i = 0usize;
    let mut neg = false;
    if i < token.len() && token[i] == b'-' {
        neg = true;
        i += 1;
    }
    let mut base: i64 = 10;
    if i + 2 <= token.len() && token[i] == b'0' && (token[i + 1] == b'x' || token[i + 1] == b'X') {
        base = 16;
        i += 2;
    }
    while i < token.len() {
        if token[i] == b'_' {
            i += 1;
            continue;
        }
        let d = match token[i] {
            b'0'..=b'9' => (token[i] - b'0') as i64,
            b'a'..=b'f' if base == 16 => 10 + (token[i] - b'a') as i64,
            b'A'..=b'F' if base == 16 => 10 + (token[i] - b'A') as i64,
            _ => break,
        };
        v = v.saturating_mul(base).saturating_add(d);
        i += 1;
    }
    Some(if neg { -v } else { v })
}


fn typecheck_word_body(
    out: &mut impl Output,
    src: &[u8],
    body_span: Span,
    declared: &WordSig,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    checks: ChecksMode,
    allow_suspend: bool,
) -> Result<(), TcError> {
    let slice = &src[body_span.start..body_span.end];
    let mut lex = Lexer::new(slice);

    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;

    // Seed stack with declared inputs.
    for i in 0..(declared.in_len as usize) {
        stack[sp] = Value::Plain(declared.inputs[i]);
        sp += 1;
    }

    let mut locals: [TypeAtom; 64] = [TypeAtom::new(b"").unwrap(); 64];
    let mut local_tys: [TypeAtom; 64] = [TypeAtom::new(b"").unwrap(); 64];
    let mut local_len: usize = 0;

    let mut terminated = false;
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            break;
        }
        if terminated {
            continue;
        }

        match tok.kind {
            TokenKind::Number => {
                push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?;
                out.write(b"  ");
                out.write(&slice[tok.span.start..tok.span.end]);
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::String => {
                push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"str").unwrap()))?;
                out.write(b"  <str> | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctArrowBind => {
                let name = lex.next();
                if name.kind != TokenKind::Ident {
                    return Err(TcError {
                        code: 3201,
                        span: Span::new(body_span.start + name.span.start, body_span.start + name.span.end),
                    });
                }
                let v = pop(&stack, &mut sp).ok_or(TcError {
                    code: 3202,
                    span: Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end),
                })?;
                if v == Value::Plain(TypeAtom::new(b"scoped").unwrap()) {
                    return Err(TcError { code: 3504, span: body_span });
                }
	                let ty = match v {
	                    Value::Plain(t) => t,
	                    Value::Scoped { ty, .. } => ty,
	                    Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                    Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                    Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                    Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                    Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                    Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                    Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                };
                let lname = TypeAtom::new(&slice[name.span.start..name.span.end]).ok_or(TcError {
                    code: 3203,
                    span: Span::new(body_span.start + name.span.start, body_span.start + name.span.end),
                })?;
                if find_local(&locals, local_len, lname).is_some() {
                    return Err(TcError {
                        code: 3204,
                        span: Span::new(body_span.start + name.span.start, body_span.start + name.span.end),
                    });
                }
                if local_len >= locals.len() {
                    return Err(TcError { code: 3205, span: body_span });
                }
                locals[local_len] = lname;
                local_tys[local_len] = ty;
                local_len += 1;
                out.write(b"  => ");
                out.write(lname.as_bytes());
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctLBracket => {
                // Capture the whole quotation span, including nested brackets.
                let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                    .map_err(|code| TcError { code, span: Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end) })?;
                let q_span = Span::new(body_span.start + q.start, body_span.start + q.end);
                push(&mut stack, &mut sp, Value::Quot(q_span))?;
                out.write(b"  <quot> | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                let mut_tok = tok.kind == TokenKind::PunctAmpBang;
                let place = parse_place(&mut lex, slice).ok_or(TcError {
                    code: 3500,
                    span: Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end),
                })?;
                let place_bytes = &slice[place.full.start..place.full.end];
                let place_abs = Span::new(body_span.start + place.full.start, body_span.start + place.full.end);

                if let Some(res) = resolve_mmio_place(mmio, src, place_bytes, place_abs)? {
                    match res {
                        MmioResolved::Reg(reg) => {
                            if mut_tok && !access_can_write(reg.access) {
                                return Err(TcError { code: 3609, span: place_abs });
                            }
                            push(
                                &mut stack,
                                &mut sp,
                                Value::MmioPtr {
                                    reg,
                                    mutable: mut_tok,
                                },
                            )?;
                        }
                        MmioResolved::Field(_) => {
                            // Fields are not addressable.
                            return Err(TcError { code: 3608, span: place_abs });
                        }
                    }
                } else {
                    if mut_tok {
                        // locals are immutable in v1
                        let root = TypeAtom::new(&slice[place.root.start..place.root.end]).unwrap();
                        if find_local(&locals, local_len, root).is_some() {
                            return Err(TcError { code: 3501, span: place.root_abs(body_span.start) });
                        }
                    }
                    let ty = if mut_tok { TypeAtom::new(b"ptr_mut").unwrap() } else { TypeAtom::new(b"ptr").unwrap() };
                    push(&mut stack, &mut sp, Value::Plain(ty))?;
                }
                out.write(b"  ");
                out.write(if mut_tok { b"&!" } else { b"&" });
                out.write(place_bytes);
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
                if sp == 0 {
                    return Err(TcError { code: 3505, span: body_span });
                }
                let top_ty = match stack[sp - 1] {
                    Value::Plain(t) => t,
                    Value::Scoped { ty, .. } => ty,
                    Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
                    Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                    Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
                    Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                    Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                    Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                    Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                };
                if array_elem_type(top_ty).is_none() && top_ty != TypeAtom::new(b"Region").unwrap() {
                    return Err(TcError { code: 3515, span: body_span });
                }

                // Non-IR checker uses the legacy marker to ensure "must be consumed by end of block".
                let scoped = Value::Plain(TypeAtom::new(b"scoped").unwrap());
                push(&mut stack, &mut sp, scoped)?;

                let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError {
                    code,
                    span: Span::new(body_span.start + tok.span.start, body_span.start + tok.span.end),
                })?;
                let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                typecheck_plain_body(
                    &mut stack,
                    &mut sp,
                    src,
                    Span::new(body_span.start + block.inner_start, body_span.start + block.inner_end),
                    env,
                    subtypes,
                    mmio,
                    checks,
                    block_allow_suspend,
                )?;

                if !check_no_scoped_live(&stack, sp) {
                    return Err(TcError { code: 3506, span: body_span });
                }
                out.write(b"  ");
                out.write(if mut_scope { b"&![...]" } else { b"&[...]" });
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            TokenKind::Ident
            | TokenKind::PunctGe
            | TokenKind::PunctLe
            | TokenKind::PunctEqEq
            | TokenKind::PunctNe => {
                let mut qbuf = [0u8; 64];
                let (name, name_span) = if tok.kind == TokenKind::Ident {
                    let (len, used, span) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                    let bytes = if used {
                        &qbuf[..len]
                    } else {
                        &slice[tok.span.start..tok.span.end]
                    };
                    (bytes, span)
                } else {
                    (&slice[tok.span.start..tok.span.end], tok.span)
                };
                if name == b"true" || name == b"false" {
                    push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                    out.write(b"  ");
                    out.write(name);
                    out.write(b" | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                let name_abs = Span::new(body_span.start + name_span.start, body_span.start + name_span.end);
                if tok.kind == TokenKind::Ident && !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
                    // Typed loads/stores: `@u32` / `!u32` and untyped `@` / `!` for MMIO places.
                    let is_load = name[0] == b'@';
                    let typed = name.len() > 1;
                    let ty_atom = if typed {
                        Some(TypeAtom::new(&name[1..]).ok_or(TcError { code: 3632, span: name_abs })?)
                    } else {
                        None
                    };

                    if is_load {
                        let addr = pop(&stack, &mut sp).ok_or(TcError { code: 3633, span: name_abs })?;
                        match addr {
                            Value::MmioPtr { reg, .. } => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError { code: 3610, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                push(&mut stack, &mut sp, Value::Plain(reg.reg_ty))?;
                                out.write(b"  ");
                                out.write(if reg.volatile { b"vol_load " } else { b"load " });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError { code: 3610, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                push(&mut stack, &mut sp, Value::Plain(reg.reg_ty))?;
                                out.write(b"  ");
                                out.write(if reg.volatile { b"vol_load " } else { b"load " });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Field(field)) => {
                                if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
                                    return Err(TcError { code: 3610, span: name_abs });
                                }
                                if typed {
                                    return Err(TcError { code: 3634, span: name_abs });
                                }
                                push(&mut stack, &mut sp, Value::Plain(field.field.ty))?;
                                let (mask, shift) = field_mask_shift(&field.field);
                                out.write(b"  ");
                                out.write(if field.volatile { b"vol_load_field " } else { b"load_field " });
                                out.write(field.field.ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, field.place_span));
                                out.write(b" mask=0x");
                                write_u64_hex(out, mask);
                                out.write(b" shift=");
                                write_u64_dec(out, shift as u64);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            Value::Plain(t) => {
                                if !typed {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                if t != TypeAtom::new(b"ptr").unwrap() && t != TypeAtom::new(b"ptr_mut").unwrap() {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                let want = ty_atom.unwrap();
                                push(&mut stack, &mut sp, Value::Plain(want))?;
                                out.write(b"  ");
                                out.write(name);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            _ => return Err(TcError { code: 3614, span: name_abs }),
                        }
                    } else {
                        let val = pop(&stack, &mut sp).ok_or(TcError { code: 3633, span: name_abs })?;
                        let addr = pop(&stack, &mut sp).ok_or(TcError { code: 3633, span: name_abs })?;
                        match (addr, val) {
                            (Value::MmioPtr { reg, mutable }, Value::Plain(vty)) => {
                                if !mutable {
                                    return Err(TcError { code: 3609, span: name_abs });
                                }
                                if !access_can_write(reg.access) {
                                    return Err(TcError { code: 3609, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError { code: 3231, span: name_abs });
                                }
                                out.write(b"  ");
                                out.write(if reg.volatile { b"vol_store " } else { b"store " });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(vty)) => {
                                if !access_can_write(reg.access) {
                                    return Err(TcError { code: 3609, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError { code: 3231, span: name_abs });
                                }
                                out.write(b"  ");
                                out.write(if reg.volatile { b"vol_store " } else { b"store " });
                                out.write(reg.reg_ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, reg.place_span));
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(vty)) => {
                                if !access_can_write(field.reg_access) || !access_can_write(field.field.access) {
                                    return Err(TcError { code: 3609, span: name_abs });
                                }
                                if typed {
                                    return Err(TcError { code: 3634, span: name_abs });
                                }
                                if vty != field.field.ty {
                                    return Err(TcError { code: 3231, span: name_abs });
                                }
                                let (mask, shift) = field_mask_shift(&field.field);
                                out.write(b"  ");
                                out.write(if field.volatile { b"vol_store_field " } else { b"store_field " });
                                out.write(field.field.ty.as_bytes());
                                out.write(b" ");
                                out.write(slice_span(src, field.place_span));
                                out.write(b" mask=0x");
                                write_u64_hex(out, mask);
                                out.write(b" shift=");
                                write_u64_dec(out, shift as u64);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            (Value::Plain(t), Value::Plain(_vty)) => {
                                if !typed {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                if t != TypeAtom::new(b"ptr_mut").unwrap() {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                out.write(b"  ");
                                out.write(name);
                                out.write(b" | stack: ");
                                write_stack(out, &stack, sp);
                                out.write(b"\n");
                                continue;
                            }
                            _ => return Err(TcError { code: 3614, span: name_abs }),
                        }
                    }
                }

                if tok.kind == TokenKind::Ident {
                    if let Some(res) = resolve_mmio_place(mmio, src, name, name_abs)? {
                        push(&mut stack, &mut sp, Value::MmioPlace(res))?;
                        out.write(b"  mmio ");
                        out.write(name);
                        out.write(b" | stack: ");
                        write_stack(out, &stack, sp);
                        out.write(b"\n");
                        continue;
                    }
                }

                if name == b"dup" {
                    let top = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    push(&mut stack, &mut sp, top)?;
                    push(&mut stack, &mut sp, top)?;
                    out.write(b"  dup | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"drop" {
                    let _ = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    out.write(b"  drop | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"swap" {
                    let b = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    let a = pop(&stack, &mut sp).ok_or(TcError { code: 3202, span: body_span })?;
                    push(&mut stack, &mut sp, b)?;
                    push(&mut stack, &mut sp, a)?;
                    out.write(b"  swap | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                if name == b"as" || name == b"as?" || name == b"bitcast" {
                    let ty = lex.next();
                    if ty.kind != TokenKind::Ident {
                        return Err(TcError {
                            code: 3295,
                            span: Span::new(body_span.start + ty.span.start, body_span.start + ty.span.end),
                        });
                    }
                    let ty_atom = TypeAtom::new(&slice[ty.span.start..ty.span.end]).ok_or(TcError {
                        code: 3296,
                        span: Span::new(body_span.start + ty.span.start, body_span.start + ty.span.end),
                    })?;
                    if name == b"as?" {
                        // ( base -- subtype ok ) for subtypes; for MVP treat others as identity + ok
                        let v = pop(&stack, &mut sp).ok_or(TcError { code: 3297, span: body_span })?;
	                        let got = match v {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
                        if let Some(st) = find_subtype(subtypes, ty_atom) {
                            if !type_compatible(got, st.base, subtypes) {
                                return Err(TcError { code: 3298, span: body_span });
                            }
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                            push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                        } else {
                            // general: assume it can convert, return ok
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                            push(&mut stack, &mut sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                        }
                        out.write(b"  ");
                        out.write(name);
                        out.write(b" ");
                        out.write(ty_atom.as_bytes());
                        out.write(b" | stack: ");
                        write_stack(out, &stack, sp);
                        out.write(b"\n");
                        continue;
                    }
                    if name == b"as" {
                        let v = pop(&stack, &mut sp).ok_or(TcError { code: 3299, span: body_span })?;
	                        let got = match v {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
                        if let Some(st) = find_subtype(subtypes, ty_atom) {
                            if !type_compatible(got, st.base, subtypes) {
                                return Err(TcError { code: 3300, span: body_span });
                            }
                            if checks == ChecksMode::All {
                                out.write(b"  check_subtype ");
                                out.write(ty_atom.as_bytes());
                                out.write(b"\n");
                                out.write(b"  trap_if_false SUBTYPE_FAIL\n");
                            }
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                        } else {
                            push(&mut stack, &mut sp, Value::Plain(ty_atom))?;
                        }
                        out.write(b"  as ");
                        out.write(ty_atom.as_bytes());
                        out.write(b" | stack: ");
                        write_stack(out, &stack, sp);
                        out.write(b"\n");
                        continue;
                    }
                }

                if name == b"if" {
                    do_if(&mut stack, &mut sp, src, env, subtypes, mmio, allow_suspend)?;
                    out.write(b"  if | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"while" {
                    do_while(&mut stack, &mut sp, src, env, subtypes, mmio, allow_suspend)?;
                    out.write(b"  while | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"loop" {
                    do_loop(&mut stack, &mut sp, src, env, subtypes, mmio, allow_suspend)?;
                    out.write(b"  loop | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"return" {
                    let want = declared.out_len as usize;
                    if sp != want {
                        return Err(TcError { code: 3230, span: body_span });
                    }
	                    for i in 0..want {
	                        let got = match stack[i] {
	                            Value::Plain(t) => t,
	                            Value::Scoped { ty, .. } => ty,
	                            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	                            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	                            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	                        };
                        if !type_compatible(got, declared.outputs[i], subtypes) {
                            return Err(TcError { code: 3231, span: body_span });
                        }
                    }
                    terminated = true;
                    out.write(b"  return | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }
                if name == b"lock" {
                    do_lock(&mut stack, &mut sp, src, env, subtypes, mmio)?;
                    out.write(b"  lock | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                if let Some(idx) = find_local(&locals, local_len, TypeAtom::new(name).unwrap_or(TypeAtom::new(b"").unwrap())) {
                    push(&mut stack, &mut sp, Value::Plain(local_tys[idx]))?;
                    out.write(b"  ");
                    out.write(name);
                    out.write(b" | stack: ");
                    write_stack(out, &stack, sp);
                    out.write(b"\n");
                    continue;
                }

                let entry = lookup(env, name).ok_or(TcError {
                    code: 3210,
                    span: Span::new(body_span.start + name_span.start, body_span.start + name_span.end),
                })?;
                if entry.may_suspend && !allow_suspend {
                    return Err(TcError { code: 3503, span: Span::new(body_span.start + name_span.start, body_span.start + name_span.end) });
                }
                apply_sig(
                    &mut stack,
                    &mut sp,
                    entry,
                    Span::new(body_span.start + name_span.start, body_span.start + name_span.end),
                    subtypes,
                )?;

                out.write(b"  ");
                out.write(name);
                out.write(b" | stack: ");
                write_stack(out, &stack, sp);
                out.write(b"\n");
            }
            _ => {
                // ignore other punctuation in MVP
            }
        }
    }

    // At end, stack must match declared outputs.
    if !check_no_scoped_live(&stack, sp) {
        return Err(TcError { code: 3504, span: body_span });
    }
    if sp != declared.out_len as usize {
        return Err(TcError { code: 3220, span: body_span });
    }
	    for i in 0..(declared.out_len as usize) {
	        let got = match stack[i] {
	            Value::Plain(t) => t,
	            Value::Scoped { ty, .. } => ty,
	            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	        };
        if !type_compatible(got, declared.outputs[i], subtypes) {
            return Err(TcError { code: 3221, span: body_span });
        }
    }

    Ok(())
}


fn do_if(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    allow_suspend: bool,
) -> Result<(), TcError> {
    let else_q = pop(stack, sp).ok_or(TcError { code: 3240, span: Span::new(0, 0) })?;
    let then_q = pop(stack, sp).ok_or(TcError { code: 3241, span: Span::new(0, 0) })?;
    let cond = pop(stack, sp).ok_or(TcError { code: 3242, span: Span::new(0, 0) })?;
    if cond != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3243, span: Span::new(0, 0) });
    }
    let then_span = match then_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3244, span: Span::new(0, 0) }),
    };
    let else_span = match else_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3245, span: Span::new(0, 0) }),
    };

    let base_sp = *sp;
    let mut then_stack = *stack;
    let mut then_sp = base_sp;
    typecheck_quote_body(&mut then_stack, &mut then_sp, src, then_span, env, subtypes, mmio, allow_suspend)?;

    let mut else_stack = *stack;
    let mut else_sp = base_sp;
    typecheck_quote_body(&mut else_stack, &mut else_sp, src, else_span, env, subtypes, mmio, allow_suspend)?;

    if then_sp != else_sp {
        return Err(TcError { code: 3246, span: Span::new(0, 0) });
    }
    for i in 0..then_sp {
        if then_stack[i] != else_stack[i] {
            return Err(TcError { code: 3247, span: Span::new(0, 0) });
        }
    }

    for i in 0..then_sp {
        stack[i] = then_stack[i];
    }
    *sp = then_sp;
    Ok(())
}


fn do_while(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    allow_suspend: bool,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3250, span: Span::new(0, 0) })?;
    let cond_q = pop(stack, sp).ok_or(TcError { code: 3251, span: Span::new(0, 0) })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3252, span: Span::new(0, 0) }),
    };
    let cond_span = match cond_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3253, span: Span::new(0, 0) }),
    };

    let base_sp = *sp;
    let base_stack = *stack;

    let mut cond_stack = base_stack;
    let mut cond_sp = base_sp;
    typecheck_quote_body(&mut cond_stack, &mut cond_sp, src, cond_span, env, subtypes, mmio, allow_suspend)?;
    if cond_sp != base_sp + 1 {
        return Err(TcError { code: 3254, span: Span::new(0, 0) });
    }
    if cond_stack[cond_sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3255, span: Span::new(0, 0) });
    }
    // must preserve original stack below bool
    for i in 0..base_sp {
        if cond_stack[i] != base_stack[i] {
            return Err(TcError { code: 3256, span: Span::new(0, 0) });
        }
    }

    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes, mmio, allow_suspend)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3257, span: Span::new(0, 0) });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3258, span: Span::new(0, 0) });
        }
    }
    Ok(())
}


fn do_loop(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    allow_suspend: bool,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3260, span: Span::new(0, 0) })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3261, span: Span::new(0, 0) }),
    };

    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes, mmio, allow_suspend)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3262, span: Span::new(0, 0) });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3263, span: Span::new(0, 0) });
        }
    }
    Ok(())
}


fn do_lock(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
) -> Result<(), TcError> {
    let body_q = pop(stack, sp).ok_or(TcError { code: 3270, span: Span::new(0, 0) })?;
    let body_span = match body_q {
        Value::Quot(s) => s,
        _ => return Err(TcError { code: 3271, span: Span::new(0, 0) }),
    };
    let base_sp = *sp;
    let base_stack = *stack;
    let mut body_stack = base_stack;
    let mut body_sp = base_sp;
    // lock is non-suspending
    typecheck_quote_body(&mut body_stack, &mut body_sp, src, body_span, env, subtypes, mmio, false)?;
    if body_sp != base_sp {
        return Err(TcError { code: 3272, span: Span::new(0, 0) });
    }
    for i in 0..base_sp {
        if body_stack[i] != base_stack[i] {
            return Err(TcError { code: 3273, span: Span::new(0, 0) });
        }
    }
    Ok(())
}


fn typecheck_quote_body(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    quot_span: Span,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    allow_suspend: bool,
) -> Result<(), TcError> {
    // Expect brackets at ends; just slice inside.
    if quot_span.end <= quot_span.start + 2 {
        return Ok(());
    }
    let inner = Span::new(quot_span.start + 1, quot_span.end - 1);
    let slice = &src[inner.start..inner.end];
    let mut lex = Lexer::new(slice);

    // Optional leading signature + effect set for escaping quotations: ignore in v1 MVP.
    if lex.next().kind == TokenKind::PunctLParen {
        // rewind not possible: manually parse again with balanced skip
        lex = Lexer::new(slice);
        let first = lex.next();
        if first.kind == TokenKind::PunctLParen {
            let _ = capture_balanced(&mut lex, slice, TokenKind::PunctLParen, TokenKind::PunctRParen, first.span.start);
            let maybe_eff = lex.next();
            if maybe_eff.kind != TokenKind::EffectSet {
                // step back not supported; ok to proceed after consuming one token too far only if it's ws, but lexer skips ws.
                // So: only treat it as effect-set if it is.
                // If it isn't, we just continue with it as first term by re-lexing from its start.
                lex = Lexer::new(&slice[maybe_eff.span.start..]);
            }
        }
    } else {
        // first token consumed; re-lex from start
        lex = Lexer::new(slice);
    }

    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            break;
        }
        match tok.kind {
            TokenKind::Number => push(stack, sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?,
            TokenKind::Ident => {
                let mut qbuf = [0u8; 64];
                let (len, used, name_span) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                let name = if used {
                    &qbuf[..len]
                } else {
                    &slice[tok.span.start..tok.span.end]
                };
                if name == b"true" || name == b"false" {
                    push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                    continue;
                }
                let abs = Span::new(inner.start + name_span.start, inner.start + name_span.end);
                if !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
                    let is_load = name[0] == b'@';
                    let typed = name.len() > 1;
                    let ty_atom = if typed {
                        Some(TypeAtom::new(&name[1..]).ok_or(TcError { code: 3632, span: abs })?)
                    } else {
                        None
                    };

                    if is_load {
                        let addr = pop(stack, sp).ok_or(TcError { code: 3633, span: abs })?;
                        match addr {
                            Value::MmioPtr { reg, .. } => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError { code: 3610, span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: abs });
                                    }
                                }
                                push(stack, sp, Value::Plain(reg.reg_ty))?;
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError { code: 3610, span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: abs });
                                    }
                                }
                                push(stack, sp, Value::Plain(reg.reg_ty))?;
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Field(field)) => {
                                if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
                                    return Err(TcError { code: 3610, span: abs });
                                }
                                if typed {
                                    return Err(TcError { code: 3634, span: abs });
                                }
                                push(stack, sp, Value::Plain(field.field.ty))?;
                                continue;
                            }
                            Value::Plain(t) => {
                                if !typed {
                                    return Err(TcError { code: 3614, span: abs });
                                }
                                if t != TypeAtom::new(b"ptr").unwrap() && t != TypeAtom::new(b"ptr_mut").unwrap() {
                                    return Err(TcError { code: 3614, span: abs });
                                }
                                push(stack, sp, Value::Plain(ty_atom.unwrap()))?;
                                continue;
                            }
                            _ => return Err(TcError { code: 3614, span: abs }),
                        }
                    } else {
                        let val = pop(stack, sp).ok_or(TcError { code: 3633, span: abs })?;
                        let addr = pop(stack, sp).ok_or(TcError { code: 3633, span: abs })?;
                        match (addr, val) {
                            (Value::MmioPtr { reg, mutable }, Value::Plain(vty)) => {
                                if !mutable || !access_can_write(reg.access) {
                                    return Err(TcError { code: 3609, span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError { code: 3231, span: abs });
                                }
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(vty)) => {
                                if !access_can_write(reg.access) {
                                    return Err(TcError { code: 3609, span: abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError { code: 3231, span: abs });
                                }
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(vty)) => {
                                if !access_can_write(field.reg_access) || !access_can_write(field.field.access) {
                                    return Err(TcError { code: 3609, span: abs });
                                }
                                if typed {
                                    return Err(TcError { code: 3634, span: abs });
                                }
                                if vty != field.field.ty {
                                    return Err(TcError { code: 3231, span: abs });
                                }
                                continue;
                            }
                            (Value::Plain(t), Value::Plain(_vty)) => {
                                if !typed {
                                    return Err(TcError { code: 3614, span: abs });
                                }
                                if t != TypeAtom::new(b"ptr_mut").unwrap() {
                                    return Err(TcError { code: 3614, span: abs });
                                }
                                continue;
                            }
                            _ => return Err(TcError { code: 3614, span: abs }),
                        }
                    }
                }

                if let Some(res) = resolve_mmio_place(mmio, src, name, abs)? {
                    push(stack, sp, Value::MmioPlace(res))?;
                    continue;
                }
                if name == b"dup" {
                    let top = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    push(stack, sp, top)?;
                    push(stack, sp, top)?;
                    continue;
                }
                if name == b"drop" {
                    let _ = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    continue;
                }
                if name == b"swap" {
                    let b = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    let a = pop(stack, sp).ok_or(TcError { code: 3282, span: quot_span })?;
                    push(stack, sp, b)?;
                    push(stack, sp, a)?;
                    continue;
                }
                if name == b"if" {
                    do_if(stack, sp, src, env, subtypes, mmio, allow_suspend)?;
                    continue;
                }
                if name == b"while" {
                    do_while(stack, sp, src, env, subtypes, mmio, allow_suspend)?;
                    continue;
                }
                if name == b"loop" {
                    do_loop(stack, sp, src, env, subtypes, mmio, allow_suspend)?;
                    continue;
                }
                if name == b"lock" {
                    do_lock(stack, sp, src, env, subtypes, mmio)?;
                    continue;
                }
                let entry = lookup(env, name).ok_or(TcError { code: 3280, span: quot_span })?;
                if entry.may_suspend && !allow_suspend {
                    return Err(TcError { code: 3503, span: quot_span });
                }
                apply_sig(stack, sp, entry, quot_span, subtypes)?;
            }
            TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                // treat as word-like operator
                let name = &slice[tok.span.start..tok.span.end];
                let entry = lookup(env, name).ok_or(TcError { code: 3280, span: quot_span })?;
                if entry.may_suspend && !allow_suspend {
                    return Err(TcError { code: 3503, span: quot_span });
                }
                apply_sig(stack, sp, entry, quot_span, subtypes)?;
            }
            TokenKind::PunctArrowBind => {
                // locals not allowed inside quotations in MVP; ignore
                return Err(TcError { code: 3281, span: quot_span });
            }
            TokenKind::PunctLBracket => {
                let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                    .map_err(|code| TcError { code, span: quot_span })?;
                let q_span = Span::new(inner.start + q.start, inner.start + q.end);
                push(stack, sp, Value::Quot(q_span))?;
            }
            TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                let mut_tok = tok.kind == TokenKind::PunctAmpBang;
                let place = parse_place(&mut lex, slice).ok_or(TcError { code: 3500, span: quot_span })?;
                let place_bytes = &slice[place.full.start..place.full.end];
                let place_abs = Span::new(inner.start + place.full.start, inner.start + place.full.end);
                if let Some(res) = resolve_mmio_place(mmio, src, place_bytes, place_abs)? {
                    match res {
                        MmioResolved::Reg(reg) => {
                            if mut_tok && !access_can_write(reg.access) {
                                return Err(TcError { code: 3609, span: place_abs });
                            }
                            push(
                                stack,
                                sp,
                                Value::MmioPtr {
                                    reg,
                                    mutable: mut_tok,
                                },
                            )?;
                        }
                        MmioResolved::Field(_) => return Err(TcError { code: 3608, span: place_abs }),
                    }
                } else {
                    let ty = if mut_tok { TypeAtom::new(b"ptr_mut").unwrap() } else { TypeAtom::new(b"ptr").unwrap() };
                    push(stack, sp, Value::Plain(ty))?;
                }
            }
            TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
                if *sp == 0 {
                    return Err(TcError { code: 3505, span: quot_span });
                }
                let top_ty = match stack[*sp - 1] {
                    Value::Plain(t) => t,
                    Value::Scoped { ty, .. } => ty,
                    Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
                    Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                    Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
                    Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                    Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                    Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                    Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                };
                if array_elem_type(top_ty).is_none() && top_ty != TypeAtom::new(b"Region").unwrap() {
                    return Err(TcError { code: 3515, span: quot_span });
                }
                let scoped = Value::Plain(TypeAtom::new(b"scoped").unwrap());
                push(stack, sp, scoped)?;
                let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span: quot_span })?;
                let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                typecheck_plain_body(
                    stack,
                    sp,
                    src,
                    Span::new(inner.start + block.inner_start, inner.start + block.inner_end),
                    env,
                    subtypes,
                    mmio,
                    ChecksMode::All,
                    block_allow_suspend,
                )?;
                if !check_no_scoped_live(stack, *sp) {
                    return Err(TcError { code: 3506, span: quot_span });
                }
            }
            _ => {}
        }
    }
    Ok(())
}


fn typecheck_plain_body(
    stack: &mut [Value; 256],
    sp: &mut usize,
    src: &[u8],
    span: Span,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    checks: ChecksMode,
    allow_suspend: bool,
) -> Result<(), TcError> {
    let slice = &src[span.start..span.end];
    let mut lex = Lexer::new(slice);
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            break;
        }
        match tok.kind {
            TokenKind::Number => push(stack, sp, Value::Plain(TypeAtom::new(b"i64").unwrap()))?,
            TokenKind::Ident | TokenKind::PunctGe | TokenKind::PunctLe | TokenKind::PunctEqEq | TokenKind::PunctNe => {
                let mut qbuf = [0u8; 64];
                let (name, name_abs) = if tok.kind == TokenKind::Ident {
                    let (len, used, s) = read_qualified_name(&mut lex, slice, tok, &mut qbuf);
                    let bytes = if used {
                        &qbuf[..len]
                    } else {
                        &slice[tok.span.start..tok.span.end]
                    };
                    (bytes, Span::new(span.start + s.start, span.start + s.end))
                } else {
                    (
                        &slice[tok.span.start..tok.span.end],
                        Span::new(span.start + tok.span.start, span.start + tok.span.end),
                    )
                };
                if name == b"true" || name == b"false" {
                    push(stack, sp, Value::Plain(TypeAtom::new(b"bool").unwrap()))?;
                    continue;
                }
                if tok.kind == TokenKind::Ident && !name.is_empty() && (name[0] == b'@' || name[0] == b'!') {
                    let is_load = name[0] == b'@';
                    let typed = name.len() > 1;
                    let ty_atom = if typed {
                        Some(TypeAtom::new(&name[1..]).ok_or(TcError { code: 3632, span: name_abs })?)
                    } else {
                        None
                    };

                    if is_load {
                        let addr = pop(stack, sp).ok_or(TcError { code: 3633, span: name_abs })?;
                        match addr {
                            Value::MmioPtr { reg, .. } => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError { code: 3610, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                push(stack, sp, Value::Plain(reg.reg_ty))?;
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Reg(reg)) => {
                                if !access_can_read(reg.access) {
                                    return Err(TcError { code: 3610, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                push(stack, sp, Value::Plain(reg.reg_ty))?;
                                continue;
                            }
                            Value::MmioPlace(MmioResolved::Field(field)) => {
                                if !access_can_read(field.reg_access) || !access_can_read(field.field.access) {
                                    return Err(TcError { code: 3610, span: name_abs });
                                }
                                if typed {
                                    return Err(TcError { code: 3634, span: name_abs });
                                }
                                push(stack, sp, Value::Plain(field.field.ty))?;
                                continue;
                            }
                            Value::Plain(t) => {
                                if !typed {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                if t != TypeAtom::new(b"ptr").unwrap() && t != TypeAtom::new(b"ptr_mut").unwrap() {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                push(stack, sp, Value::Plain(ty_atom.unwrap()))?;
                                continue;
                            }
                            _ => return Err(TcError { code: 3614, span: name_abs }),
                        }
                    } else {
                        let val = pop(stack, sp).ok_or(TcError { code: 3633, span: name_abs })?;
                        let addr = pop(stack, sp).ok_or(TcError { code: 3633, span: name_abs })?;
                        match (addr, val) {
                            (Value::MmioPtr { reg, mutable }, Value::Plain(vty)) => {
                                if !mutable || !access_can_write(reg.access) {
                                    return Err(TcError { code: 3609, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError { code: 3231, span: name_abs });
                                }
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Reg(reg)), Value::Plain(vty)) => {
                                if !access_can_write(reg.access) {
                                    return Err(TcError { code: 3609, span: name_abs });
                                }
                                if let Some(want) = ty_atom {
                                    if want != reg.reg_ty {
                                        return Err(TcError { code: 3613, span: name_abs });
                                    }
                                }
                                if vty != reg.reg_ty {
                                    return Err(TcError { code: 3231, span: name_abs });
                                }
                                continue;
                            }
                            (Value::MmioPlace(MmioResolved::Field(field)), Value::Plain(vty)) => {
                                if !access_can_write(field.reg_access) || !access_can_write(field.field.access) {
                                    return Err(TcError { code: 3609, span: name_abs });
                                }
                                if typed {
                                    return Err(TcError { code: 3634, span: name_abs });
                                }
                                if vty != field.field.ty {
                                    return Err(TcError { code: 3231, span: name_abs });
                                }
                                continue;
                            }
                            (Value::Plain(t), Value::Plain(_vty)) => {
                                if !typed {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                if t != TypeAtom::new(b"ptr_mut").unwrap() {
                                    return Err(TcError { code: 3614, span: name_abs });
                                }
                                continue;
                            }
                            _ => return Err(TcError { code: 3614, span: name_abs }),
                        }
                    }
                }
                if tok.kind == TokenKind::Ident {
                    if let Some(res) = resolve_mmio_place(mmio, src, name, name_abs)? {
                        push(stack, sp, Value::MmioPlace(res))?;
                        continue;
                    }
                }
                if name == b"dup" {
                    let top = pop(stack, sp).ok_or(TcError { code: 3282, span })?;
                    push(stack, sp, top)?;
                    push(stack, sp, top)?;
                    continue;
                }
                if name == b"drop" {
                    let _ = pop(stack, sp).ok_or(TcError { code: 3282, span })?;
                    continue;
                }
                if name == b"swap" {
                    let b = pop(stack, sp).ok_or(TcError { code: 3282, span })?;
                    let a = pop(stack, sp).ok_or(TcError { code: 3282, span })?;
                    push(stack, sp, b)?;
                    push(stack, sp, a)?;
                    continue;
                }
                if name == b"if" {
                    do_if(stack, sp, src, env, subtypes, mmio, allow_suspend)?;
                    continue;
                }
                if name == b"while" {
                    do_while(stack, sp, src, env, subtypes, mmio, allow_suspend)?;
                    continue;
                }
                if name == b"loop" {
                    do_loop(stack, sp, src, env, subtypes, mmio, allow_suspend)?;
                    continue;
                }
                if name == b"lock" {
                    do_lock(stack, sp, src, env, subtypes, mmio)?;
                    continue;
                }
                if name == b"as" || name == b"as?" || name == b"bitcast" {
                    // Defer to outer parser: here we just ignore and let mismatch show later.
                    continue;
                }
                let entry = lookup(env, name).ok_or(TcError { code: 3280, span })?;
                if entry.may_suspend && !allow_suspend {
                    return Err(TcError { code: 3503, span });
                }
                apply_sig(stack, sp, entry, span, subtypes)?;
                let _ = checks;
            }
            TokenKind::PunctLBracket => {
                let q = capture_balanced(&mut lex, slice, TokenKind::PunctLBracket, TokenKind::PunctRBracket, tok.span.start)
                    .map_err(|code| TcError { code, span })?;
                let q_span = Span::new(span.start + q.start, span.start + q.end);
                push(stack, sp, Value::Quot(q_span))?;
            }
            TokenKind::PunctAmp | TokenKind::PunctAmpBang => {
                let mut_tok = tok.kind == TokenKind::PunctAmpBang;
                let place = parse_place(&mut lex, slice).ok_or(TcError { code: 3500, span })?;
                let place_bytes = &slice[place.full.start..place.full.end];
                let place_abs = Span::new(span.start + place.full.start, span.start + place.full.end);
                if let Some(res) = resolve_mmio_place(mmio, src, place_bytes, place_abs)? {
                    match res {
                        MmioResolved::Reg(reg) => {
                            if mut_tok && !access_can_write(reg.access) {
                                return Err(TcError { code: 3609, span: place_abs });
                            }
                            push(
                                stack,
                                sp,
                                Value::MmioPtr {
                                    reg,
                                    mutable: mut_tok,
                                },
                            )?;
                        }
                        MmioResolved::Field(_) => return Err(TcError { code: 3608, span: place_abs }),
                    }
                } else {
                    let ty = if mut_tok { TypeAtom::new(b"ptr_mut").unwrap() } else { TypeAtom::new(b"ptr").unwrap() };
                    push(stack, sp, Value::Plain(ty))?;
                }
            }
            TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                let mut_scope = tok.kind == TokenKind::PunctAmpBangLBracket;
                if *sp == 0 {
                    return Err(TcError { code: 3505, span });
                }
                let top_ty = match stack[*sp - 1] {
                    Value::Plain(t) => t,
                    Value::Scoped { ty, .. } => ty,
                    Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
                    Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                    Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
                    Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                    Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                    Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
                    Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
                };
                if array_elem_type(top_ty).is_none() && top_ty != TypeAtom::new(b"Region").unwrap() {
                    return Err(TcError { code: 3515, span });
                }
                let scoped = Value::Plain(TypeAtom::new(b"scoped").unwrap());
                push(stack, sp, scoped)?;
                let block = capture_scoped_block(&mut lex, slice, tok.span).map_err(|code| TcError { code, span })?;
                let block_allow_suspend = if mut_scope { false } else { allow_suspend };
                typecheck_plain_body(
                    stack,
                    sp,
                    src,
                    Span::new(span.start + block.inner_start, span.start + block.inner_end),
                    env,
                    subtypes,
                    mmio,
                    checks,
                    block_allow_suspend,
                )?;
                if !check_no_scoped_live(stack, *sp) {
                    return Err(TcError { code: 3506, span });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

struct PlaceSpans {
    root: Span,
    full: Span,
}

impl PlaceSpans {
    fn root_abs(&self, base: usize) -> Span {
        Span::new(base + self.root.start, base + self.root.end)
    }
}

fn parse_place(lex: &mut Lexer<'_>, slice: &[u8]) -> Option<PlaceSpans> {
    let mut probe = *lex;
    let first = probe.next();
    if first.kind != TokenKind::Ident {
        return None;
    }
    let root = first.span;
    let mut end = first.span.end;
    loop {
        let mut probe2 = probe;
        let dot = probe2.next();
        if dot.kind != TokenKind::PunctDot {
            break;
        }
        let seg = probe2.next();
        if seg.kind != TokenKind::Ident {
            return None;
        }
        end = seg.span.end;
        probe = probe2;
    }
    *lex = probe;
    let full = Span::new(root.start, end);
    let _ = slice;
    Some(PlaceSpans { root, full })
}

fn read_qualified_name(
    lex: &mut Lexer<'_>,
    slice: &[u8],
    first: frontend::token::Token,
    buf: &mut [u8; 64],
) -> (usize, bool, Span) {
    let mut probe = *lex;
    let mut used = false;

    let first_bytes = &slice[first.span.start..first.span.end];
    if first_bytes.len() <= buf.len() {
        buf[..first_bytes.len()].copy_from_slice(first_bytes);
    } else {
        return (0, false, first.span);
    }
    let mut len = first_bytes.len();

    let mut end = first.span.end;
    loop {
        let mut probe2 = probe;
        let dot = probe2.next();
        if dot.kind != TokenKind::PunctDot {
            break;
        }
        let seg = probe2.next();
        if seg.kind != TokenKind::Ident {
            break;
        }
        let seg_bytes = &slice[seg.span.start..seg.span.end];
        if len + 1 + seg_bytes.len() > buf.len() {
            break;
        }
        buf[len] = b'.';
        len += 1;
        buf[len..len + seg_bytes.len()].copy_from_slice(seg_bytes);
        len += seg_bytes.len();
        end = seg.span.end;
        used = true;
        probe = probe2;
    }
    if used {
        *lex = probe;
    }
    if used {
        (len, true, Span::new(first.span.start, end))
    } else {
        (0, false, first.span)
    }
}

struct ScopedBlock {
    inner_start: usize,
    inner_end: usize,
}

fn capture_scoped_block(lex: &mut Lexer<'_>, slice: &[u8], open_span: Span) -> Result<ScopedBlock, u32> {
    let mut depth = 1usize;
    let inner_start = open_span.end;
    loop {
        let t = lex.next();
        match t.kind {
            TokenKind::Eof => return Err(3590),
            TokenKind::PunctRBracket => {
                depth -= 1;
                if depth == 0 {
                    let inner_end = t.span.start;
                    let _ = slice;
                    return Ok(ScopedBlock { inner_start, inner_end });
                }
            }
            TokenKind::PunctLBracket | TokenKind::PunctAmpLBracket | TokenKind::PunctAmpBangLBracket => {
                depth += 1;
            }
            _ => {}
        }
        let _ = slice;
    }
}

fn lookup<'a>(env: &'a [WordEntry], name: &[u8]) -> Option<&'a WordEntry> {
    for e in env {
        if e.name.as_bytes() == name {
            return Some(e);
        }
    }
    None
}

fn apply_sig(
    stack: &mut [Value; 256],
    sp: &mut usize,
    entry: &WordEntry,
    span: Span,
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    if entry.may_suspend {
        if !check_no_scoped_live(stack, *sp) {
            return Err(TcError { code: 3502, span });
        }
    }

    let sig = &entry.sig;
    let need = sig.in_len as usize;
    if *sp < need {
        return Err(TcError { code: 3211, span });
    }
    // Check types from top.
	    for i in 0..need {
	        let got = stack[*sp - need + i];
	        let got = match got {
	            Value::Plain(t) => t,
	            Value::Scoped { ty, .. } => ty,
	            Value::Resource(_) => TypeAtom::new(b"resource").unwrap(),
	            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
	            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
	            Value::Ptr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::Ptr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	            Value::MmioPtr { mutable: false, .. } => TypeAtom::new(b"ptr").unwrap(),
	            Value::MmioPtr { mutable: true, .. } => TypeAtom::new(b"ptr_mut").unwrap(),
	        };
        if !type_compatible(got, sig.inputs[i], subtypes) {
            return Err(TcError { code: 3212, span });
        }
    }
    *sp -= need;
    for i in 0..(sig.out_len as usize) {
        push(stack, sp, Value::Plain(sig.outputs[i]))?;
    }
    Ok(())
}

fn check_no_scoped_live(stack: &[Value; 256], sp: usize) -> bool {
    let scoped = Value::Plain(TypeAtom::new(b"scoped").unwrap());
    let mut i = 0usize;
    while i < sp {
        if stack[i] == scoped {
            return false;
        }
        if matches!(stack[i], Value::Scoped { .. }) {
            return false;
        }
        i += 1;
    }
    true
}

fn push(stack: &mut [Value; 256], sp: &mut usize, v: Value) -> Result<(), TcError> {
    if *sp >= stack.len() {
        return Err(TcError { code: 3206, span: Span::new(0, 0) });
    }
    stack[*sp] = v;
    *sp += 1;
    Ok(())
}

fn pop(stack: &[Value; 256], sp: &mut usize) -> Option<Value> {
    if *sp == 0 {
        return None;
    }
    *sp -= 1;
    Some(stack[*sp])
}

fn find_local(locals: &[TypeAtom; 64], len: usize, name: TypeAtom) -> Option<usize> {
    let mut i = 0usize;
    while i < len {
        if locals[i] == name {
            return Some(i);
        }
        i += 1;
    }
    None
}


fn write_sig(out: &mut impl Output, sig: &WordSig) {
    out.write(b"( ");
    for i in 0..(sig.in_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        out.write(sig.inputs[i].as_bytes());
    }
    out.write(b" --");
    if sig.out_len > 0 {
        out.write(b" ");
    }
    for i in 0..(sig.out_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        out.write(sig.outputs[i].as_bytes());
    }
    out.write(b" )");
}


fn write_stack(out: &mut impl Output, stack: &[Value; 256], sp: usize) {
    for i in 0..sp {
        if i != 0 {
            out.write(b" ");
        }
	        match stack[i] {
	            Value::Plain(t) => out.write(t.as_bytes()),
	            Value::Scoped { ty, .. } => out.write(ty.as_bytes()),
	            Value::Resource(name) => out.write(name.as_bytes()),
	            Value::Quot(_) => out.write(b"quot"),
	            Value::MmioPlace(_) => out.write(b"mmio"),
	            Value::Ptr { mutable: false, .. } => out.write(b"ptr"),
	            Value::Ptr { mutable: true, .. } => out.write(b"ptr_mut"),
	            Value::MmioPtr { mutable: false, .. } => out.write(b"mmio_ptr"),
	            Value::MmioPtr { mutable: true, .. } => out.write(b"mmio_ptr_mut"),
	        }
	    }
}


fn write_u64_dec(out: &mut impl Output, mut v: u64) {
    let mut buf = [0u8; 20];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            buf[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}


fn write_u64_hex(out: &mut impl Output, mut v: u64) {
    let mut buf = [0u8; 16];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        while v > 0 && n < buf.len() {
            let d = (v & 0xF) as u8;
            buf[n] = match d {
                0..=9 => b'0' + d,
                _ => b'a' + (d - 10),
            };
            n += 1;
            v >>= 4;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

fn capture_balanced(
    lex: &mut Lexer<'_>,
    slice: &[u8],
    open: TokenKind,
    close: TokenKind,
    open_start: usize,
) -> Result<Span, u32> {
    let mut depth = 1usize;
    let mut end = open_start + 1;
    while depth > 0 {
        let t = lex.next();
        if t.kind == TokenKind::Eof {
            return Err(3290);
        }
        end = t.span.end;
        if t.kind == open {
            depth += 1;
        } else if t.kind == close {
            depth -= 1;
        } else if t.kind == TokenKind::String {
            // already handled in lexer
        }
        let _ = slice;
    }
    Ok(Span::new(open_start, end))
}

fn slice_span<'a>(src: &'a [u8], span: Span) -> &'a [u8] {
    &src[span.start..span.end]
}

fn find_subtype(subtypes: &[SubtypeInfo], name: TypeAtom) -> Option<SubtypeInfo> {
    for &s in subtypes {
        if s.name == name {
            return Some(s);
        }
    }
    None
}


fn type_compatible(got: TypeAtom, want: TypeAtom, subtypes: &[SubtypeInfo]) -> bool {
    if got == want {
        return true;
    }
    // subtype <: base
    if let Some(st) = find_subtype(subtypes, got) {
        if st.base == want {
            return true;
        }
    }
    false
}

fn array_elem_type(ty: TypeAtom) -> Option<TypeAtom> {
    let b = ty.as_bytes();
    if !b.starts_with(b"Array(") {
        return None;
    }
    if b.last().copied()? != b')' {
        return None;
    }
    let inner = &b[b"Array(".len()..b.len() - 1];
    let mut depth = 0u32;
    for (i, &c) in inner.iter().enumerate() {
        match c {
            b'(' => depth = depth.wrapping_add(1),
            b')' => depth = depth.wrapping_sub(1),
            b',' if depth == 0 => {
                let elem = &inner[..i];
                return TypeAtom::new(elem);
            }
            _ => {}
        }
    }
    None
}

fn chan_elem_type(ty: TypeAtom) -> Option<TypeAtom> {
    let b = ty.as_bytes();
    if !b.starts_with(b"Chan(") {
        return None;
    }
    if b.last().copied()? != b')' {
        return None;
    }
    let inner = &b[b"Chan(".len()..b.len() - 1];
    TypeAtom::new(inner)
}

fn slice_type_of_elem(elem: TypeAtom, mutable: bool) -> Option<TypeAtom> {
    let mut buf = [0u8; 32];
    let mut k = 0usize;
    if mutable {
        k = push_bytes(&mut buf, k, b"SliceMut(")?;
    } else {
        k = push_bytes(&mut buf, k, b"Slice(")?;
    }
    k = push_bytes(&mut buf, k, elem.as_bytes())?;
    k = push_bytes(&mut buf, k, b")")?;
    TypeAtom::new(&buf[..k])
}

fn region_ref_type(mutable: bool) -> TypeAtom {
    if mutable {
        TypeAtom::new(b"RegionRefMut").unwrap()
    } else {
        TypeAtom::new(b"RegionRef").unwrap()
    }
}
