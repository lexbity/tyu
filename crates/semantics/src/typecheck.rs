use crate::types::{SigParseError, TypeAtom, WordEntry, WordSig};
use frontend::{fixed::FixedVec, lex::Lexer, parse::DeclKind, parse::ModuleAst, span::Span, token::TokenKind};

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
    Quot(Span),
    MmioPlace(MmioResolved),
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
}

struct MmioDb {
    maps: FixedVec<MmioMapDecl, 16>,
    instances: FixedVec<MmioInstance, 64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmioRegInfo {
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
    place_span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MmioResolved {
    Reg(MmioResolvedReg),
    Field(MmioResolvedField),
}

pub fn parse_word_sig(src: &[u8], sig_span: Span) -> Result<WordSig, SigParseError> {
    let slice = &src[sig_span.start..sig_span.end];
    let mut lex = Lexer::new(slice);
    let mut sig = WordSig::empty();
    let mut in_phase = true;

    loop {
        let t = lex.next();
        match t.kind {
            TokenKind::Eof => break,
            TokenKind::PunctLParen | TokenKind::PunctRParen => continue,
            TokenKind::PunctDashDash => {
                in_phase = false;
            }
            TokenKind::Ident => {
                let b = &slice[t.span.start..t.span.end];
                let atom = TypeAtom::new(b).ok_or(SigParseError {
                    code: 3101,
                    span: Span::new(sig_span.start + t.span.start, sig_span.start + t.span.end),
                })?;
                if in_phase {
                    let idx = sig.in_len as usize;
                    if idx >= sig.inputs.len() {
                        return Err(SigParseError {
                            code: 3102,
                            span: sig_span,
                        });
                    }
                    sig.inputs[idx] = atom;
                    sig.in_len += 1;
                } else {
                    let idx = sig.out_len as usize;
                    if idx >= sig.outputs.len() {
                        return Err(SigParseError {
                            code: 3103,
                            span: sig_span,
                        });
                    }
                    sig.outputs[idx] = atom;
                    sig.out_len += 1;
                }
            }
            _ => {
                return Err(SigParseError {
                    code: 3100,
                    span: Span::new(sig_span.start + t.span.start, sig_span.start + t.span.end),
                });
            }
        }
    }

    Ok(sig)
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
        let _ = db.instances.push(MmioInstance { name, map });
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
        Ok(Some(MmioResolved::Field(MmioResolvedField {
            map: map_decl.name,
            reg: reg_info.reg,
            reg_ty: reg_info.reg_ty,
            reg_access: reg_info.access,
            field,
            volatile: reg_info.volatile,
            place_span,
        })))
    } else {
        Ok(Some(MmioResolved::Reg(MmioResolvedReg {
            map: map_decl.name,
            reg: reg_info.reg,
            reg_ty: reg_info.reg_ty,
            access: reg_info.access,
            volatile: reg_info.volatile,
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
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mmio = build_mmio_db(module, src)?;

    out.write(b"module ");
    out.write(slice_span(src, module.name));
    out.write(b"\n");

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::Word {
            continue;
        }
        let Some(sig_span) = decl.sig else {
            return Err(TcError {
                code: 3200,
                span: decl.name,
            });
        };
        let sig = parse_word_sig(src, sig_span).map_err(|e| TcError {
            code: e.code,
            span: e.span,
        })?;
        let name_bytes = slice_span(src, decl.name);
        out.write(b"word ");
        out.write(name_bytes);
        out.write(b" ");
        write_sig(out, &sig);
        out.write(b"\n");

        if checks == ChecksMode::All {
            for i in 0..(sig.in_len as usize) {
                if is_subtype(subtypes, sig.inputs[i]) {
                    out.write(b"  check_param ");
                    out.write(sig.inputs[i].as_bytes());
                    out.write(b"\n");
                    out.write(b"  trap_if_false SUBTYPE_FAIL\n");
                }
            }
        }

        if checks != ChecksMode::Off {
            if checks == ChecksMode::Contracts || checks == ChecksMode::All {
                if let Some(req) = decl.requires {
                    out.write(b"  requires ");
                    out.write(b"[...]");
                    out.write(b"\n");
                    check_contract_predicate(src, req, &sig, env, subtypes, &mmio, out)?;
                }
            }
        }

        let Some(body_span) = decl.body else {
            continue;
        };
        typecheck_word_body(out, src, body_span, &sig, env, subtypes, &mmio, checks, true)?;

        if checks != ChecksMode::Off {
            if checks == ChecksMode::Contracts || checks == ChecksMode::All {
                if let Some(ens) = decl.ensures {
                    out.write(b"  ensures ");
                    out.write(b"[...]");
                    out.write(b"\n");
                    check_ensures_predicate(src, ens, &sig, env, subtypes, &mmio, out)?;
                }
            }
        }

        if checks == ChecksMode::All {
            for i in 0..(sig.out_len as usize) {
                if is_subtype(subtypes, sig.outputs[i]) {
                    out.write(b"  check_ret ");
                    out.write(sig.outputs[i].as_bytes());
                    out.write(b"\n");
                    out.write(b"  trap_if_false SUBTYPE_FAIL\n");
                }
            }
        }
    }

    Ok(())
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
                    Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                    Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
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
                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
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
                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
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
                            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
                            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
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
            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
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
        let dot = probe.next();
        if dot.kind != TokenKind::PunctDot {
            break;
        }
        let seg = probe.next();
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
            Value::Quot(_) => TypeAtom::new(b"quot").unwrap(),
            Value::MmioPlace(_) => TypeAtom::new(b"mmio").unwrap(),
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
            Value::Quot(_) => out.write(b"quot"),
            Value::MmioPlace(_) => out.write(b"mmio"),
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

fn is_subtype(subtypes: &[SubtypeInfo], ty: TypeAtom) -> bool {
    find_subtype(subtypes, ty).is_some()
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

fn check_contract_predicate(
    src: &[u8],
    quot_span: Span,
    sig: &WordSig,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    out: &mut impl Output,
) -> Result<(), TcError> {
    // Seed stack with declared inputs, and require predicate ends with (inputs + bool).
    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;
    for i in 0..(sig.in_len as usize) {
        stack[sp] = Value::Plain(sig.inputs[i]);
        sp += 1;
    }
    let base_sp = sp;
    typecheck_quote_body(&mut stack, &mut sp, src, quot_span, env, subtypes, mmio, false)?;
    if sp != base_sp + 1 {
        return Err(TcError { code: 3310, span: quot_span });
    }
    if stack[sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3311, span: quot_span });
    }
    // Must preserve types below bool.
    for i in 0..base_sp {
        if stack[i] != Value::Plain(sig.inputs[i]) {
            return Err(TcError { code: 3312, span: quot_span });
        }
    }
    out.write(b"  trap_if_false CONTRACT_FAIL\n");
    Ok(())
}

fn check_ensures_predicate(
    src: &[u8],
    quot_span: Span,
    sig: &WordSig,
    env: &[WordEntry],
    subtypes: &[SubtypeInfo],
    mmio: &MmioDb,
    out: &mut impl Output,
) -> Result<(), TcError> {
    let mut stack: [Value; 256] = [Value::Plain(TypeAtom::new(b"").unwrap()); 256];
    let mut sp: usize = 0;
    for i in 0..(sig.out_len as usize) {
        stack[sp] = Value::Plain(sig.outputs[i]);
        sp += 1;
    }
    let base_sp = sp;
    typecheck_quote_body(&mut stack, &mut sp, src, quot_span, env, subtypes, mmio, false)?;
    if sp != base_sp + 1 {
        return Err(TcError { code: 3320, span: quot_span });
    }
    if stack[sp - 1] != Value::Plain(TypeAtom::new(b"bool").unwrap()) {
        return Err(TcError { code: 3321, span: quot_span });
    }
    for i in 0..base_sp {
        if stack[i] != Value::Plain(sig.outputs[i]) {
            return Err(TcError { code: 3322, span: quot_span });
        }
    }
    out.write(b"  trap_if_false CONTRACT_FAIL\n");
    Ok(())
}
