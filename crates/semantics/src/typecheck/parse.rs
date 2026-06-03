use crate::typecheck::util::{parse_u32_any, push_bytes, push_u32_dec};
use crate::types::{SigParseError, TypeAtom, WordSig};
use frontend::lex::Lexer;
use frontend::span::Span;
use frontend::token::{Token, TokenKind};

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
            span: Span::new(
                sig_span.start + start,
                sig_span.start + core::cmp::min(start + 1, slice.len()),
            ),
        })?;
        i = next;

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

    Ok(sig)
}

pub fn parse_type_expr(slice: &[u8], mut i: usize) -> Option<(TypeAtom, usize)> {
    // skip ws
    while i < slice.len() && matches!(slice[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    if i >= slice.len() {
        return None;
    }
    // Pointer prefixes: ^T / ^!T
    if slice[i] == b'^' {
        let mut j = i + 1;
        let mut mutable = false;
        if j < slice.len() && slice[j] == b'!' {
            mutable = true;
            j += 1;
        }
        let (_, next) = parse_type_expr(slice, j)?;
        let atom = if mutable {
            TypeAtom::new(b"ptr_mut")?
        } else {
            TypeAtom::new(b"ptr")?
        };
        return Some((atom, next));
    }

    // Channel type: |T|
    if slice[i] == b'|' {
        let (inner, mut j) = parse_type_expr(slice, i + 1)?;
        while j < slice.len() && matches!(slice[j], b' ' | b'\n' | b'\r' | b'\t') {
            j += 1;
        }
        if j >= slice.len() || slice[j] != b'|' {
            return None;
        }
        j += 1;
        let mut buf = [0u8; 32];
        let mut k = 0usize;
        k = push_bytes(&mut buf, k, b"Chan(")?;
        k = push_bytes(&mut buf, k, inner.as_bytes())?;
        k = push_bytes(&mut buf, k, b")")?;
        let atom = TypeAtom::new(&buf[..k])?;
        if let Some((arr, next)) = parse_array_suffix(slice, atom, j) {
            return Some((arr, next));
        }
        return Some((atom, j));
    }

    // Grouped type: (T)
    if slice[i] == b'(' {
        let (inner, mut j) = parse_type_expr(slice, i + 1)?;
        while j < slice.len() && matches!(slice[j], b' ' | b'\n' | b'\r' | b'\t') {
            j += 1;
        }
        if j >= slice.len() || slice[j] != b')' {
            return None;
        }
        j += 1;
        if let Some((arr, next)) = parse_array_suffix(slice, inner, j) {
            return Some((arr, next));
        }
        return Some((inner, j));
    }

    let ident_start = i;
    while i < slice.len() {
        let b = slice[i];
        if matches!(
            b,
            b'(' | b')' | b',' | b'\'' | b'|' | b' ' | b'\n' | b'\r' | b'\t'
        ) {
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
        return None;
    }

    let atom = TypeAtom::new(name)?;
    if let Some((arr, next)) = parse_array_suffix(slice, atom, i) {
        return Some((arr, next));
    }
    Some((atom, i))
}

fn parse_array_suffix(slice: &[u8], base: TypeAtom, mut i: usize) -> Option<(TypeAtom, usize)> {
    while i < slice.len() && matches!(slice[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    if i >= slice.len() || slice[i] != b'\'' {
        return None;
    }
    i += 1;
    while i < slice.len() && matches!(slice[i], b' ' | b'\n' | b'\r' | b'\t') {
        i += 1;
    }
    let num_start = i;
    while i < slice.len() && matches!(slice[i], b'0'..=b'9' | b'_') {
        i += 1;
    }
    if num_start == i {
        return None;
    }
    let n_bytes = &slice[num_start..i];
    let n = parse_u32_any(n_bytes)?;
    let mut buf = [0u8; 32];
    let mut k = 0usize;
    k = push_bytes(&mut buf, k, b"Array(")?;
    k = push_bytes(&mut buf, k, base.as_bytes())?;
    k = push_bytes(&mut buf, k, b",")?;
    k = push_u32_dec(&mut buf, k, n)?;
    k = push_bytes(&mut buf, k, b")")?;
    let atom = TypeAtom::new(&buf[..k])?;
    Some((atom, i))
}

pub fn read_qualified_name(
    lex: &mut Lexer<'_>,
    slice: &[u8],
    first: Token,
    buf: &mut [u8; 64],
) -> (usize, bool, Span) {
    let mut probe = *lex;
    let mut used = false;

    let first_bytes = &slice[first.span.start..first.span.end];
    if first_bytes.len() <= buf.len() {
        for (i, &b) in first_bytes.iter().enumerate() {
            buf[i] = b;
        }
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
        for (i, &b) in seg_bytes.iter().enumerate() {
            buf[len + i] = b;
        }
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

pub struct ScopedBlock {
    pub inner_start: usize,
    pub inner_end: usize,
}

pub fn capture_scoped_block(
    lex: &mut Lexer<'_>,
    slice: &[u8],
    open_span: Span,
) -> Result<ScopedBlock, u32> {
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
                    return Ok(ScopedBlock {
                        inner_start,
                        inner_end,
                    });
                }
            }
            TokenKind::PunctLBracket
            | TokenKind::PunctAmpLBracket
            | TokenKind::PunctAmpBangLBracket => {
                depth += 1;
            }
            _ => {}
        }
        let _ = slice;
    }
}

pub fn capture_balanced(
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

pub struct PlaceSpans {
    pub full: Span,
    pub root: Span,
}

impl PlaceSpans {
    pub fn root_abs(&self, base: usize) -> Span {
        Span::new(base + self.root.start, base + self.root.end)
    }
}

pub fn parse_place(lex: &mut Lexer<'_>, slice: &[u8]) -> Option<PlaceSpans> {
    let mut probe = *lex;
    let first = probe.next();
    if first.kind != TokenKind::Ident {
        return None;
    }
    let root = first.span;
    let mut end = first.span.end;
    loop {
        let mut probe2 = probe;
        let next = probe2.next();
        if next.kind == TokenKind::PunctDot {
            let seg = probe2.next();
            if seg.kind != TokenKind::Ident {
                return None;
            }
            end = seg.span.end;
            probe = probe2;
            continue;
        }
        if next.kind == TokenKind::PunctApostrophe {
            let num = probe2.next();
            if num.kind != TokenKind::Number {
                return None;
            }
            end = num.span.end;
            probe = probe2;
            continue;
        }
        let _ = slice;
        break;
    }
    *lex = probe;
    let full = Span::new(root.start, end);
    Some(PlaceSpans { root, full })
}
