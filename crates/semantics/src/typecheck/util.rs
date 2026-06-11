use crate::typecheck::db::{NominalDb, SubtypeInfo};
use crate::typecheck::error::{Output, TcError};
use crate::typecheck::value::Value;
use crate::types::{TypeAtom, WordEntry, WordSig};
use frontend::span::Span;
use ir::EffectSet;

pub fn push(stack: &mut [Value; 256], sp: &mut usize, v: Value) -> Result<(), TcError> {
    if *sp >= stack.len() {
        return Err(TcError::StackOverflow {
            span: Span::UNKNOWN,
        });
    }
    stack[*sp] = v;
    *sp += 1;
    Ok(())
}

pub fn pop(stack: &[Value; 256], sp: &mut usize) -> Option<Value> {
    if *sp == 0 {
        return None;
    }
    *sp -= 1;
    Some(stack[*sp])
}

pub fn check_no_scoped_live(stack: &[Value; 256], sp: usize) -> bool {
    let scoped = Value::Plain(TypeAtom::SCOPED);
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

pub fn lookup<'a>(env: &'a [WordEntry], name: &[u8]) -> Option<&'a WordEntry> {
    env.iter().find(|e| e.name.as_bytes() == name)
}

pub fn find_local(locals: &[TypeAtom; 64], len: usize, name: TypeAtom) -> Option<usize> {
    let mut i = 0usize;
    while i < len {
        if locals[i] == name {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub fn find_subtype(subtypes: &[SubtypeInfo], name: TypeAtom) -> Option<SubtypeInfo> {
    subtypes.iter().find(|s| s.name == name).copied()
}

pub fn type_compatible(got: TypeAtom, want: TypeAtom, subtypes: &[SubtypeInfo]) -> bool {
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

pub fn slice_span(src: &[u8], span: Span) -> &[u8] {
    &src[span.start..span.end]
}

pub fn push_bytes(buf: &mut [u8; 32], mut at: usize, bytes: &[u8]) -> Option<usize> {
    if at + bytes.len() > buf.len() {
        return None;
    }
    for &b in bytes {
        buf[at] = b;
        at += 1;
    }
    Some(at)
}

pub fn push_u32_dec(buf: &mut [u8; 32], at: usize, mut v: u32) -> Option<usize> {
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

pub fn parse_u32_any(bytes: &[u8]) -> Option<u32> {
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

pub fn array_elem_type(ty: TypeAtom) -> Option<TypeAtom> {
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

pub fn array_len(ty: TypeAtom) -> Option<u32> {
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
                let n = &inner[i + 1..];
                return parse_u32_any(n);
            }
            _ => {}
        }
    }
    None
}

pub fn slice_type_of_elem(elem: TypeAtom, mutable: bool) -> Option<TypeAtom> {
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

pub fn region_ref_type(mutable: bool) -> TypeAtom {
    if mutable {
        TypeAtom::new(b"RegionRefMut").unwrap()
    } else {
        TypeAtom::new(b"RegionRef").unwrap()
    }
}

pub fn type_size_bytes(ty: TypeAtom, nominals: &NominalDb) -> Option<u32> {
    type_size_bytes_rec(ty, nominals, 8)
}

fn type_size_bytes_rec(ty: TypeAtom, nominals: &NominalDb, depth: u8) -> Option<u32> {
    if depth == 0 {
        return None;
    }
    let b = ty.as_bytes();
    let prim = match b {
        b"u8" | b"i8" | b"bool" => Some(1),
        b"u16" | b"i16" => Some(2),
        b"u32" | b"i32" => Some(4),
        b"u64" | b"i64" | b"usize" | b"isize" => Some(8),
        b"ptr" | b"ptr_mut" | b"mmio" | b"str" | b"resource" | b"quot" => Some(8),
        b"Region" | b"RegionRef" | b"RegionRefMut" | b"Task" => Some(8),
        _ => None,
    };
    if prim.is_some() {
        return prim;
    }

    if b.starts_with(b"Chan(") {
        return Some(8);
    }
    if b.starts_with(b"Slice(") || b.starts_with(b"SliceMut(") {
        return Some(16);
    }
    if let Some(bytes) = array_size_bytes(b, nominals, depth) {
        return Some(bytes);
    }

    for s in nominals.structs.iter() {
        if s.name != ty {
            continue;
        }
        let mut offset: u32 = 0;
        let mut max_align: u32 = 1;
        for f in s.fields.iter() {
            let fsize = type_size_bytes_rec(f.ty, nominals, depth - 1)?;
            let falign = field_align(fsize);
            offset = align_up(offset, falign);
            offset = offset.checked_add(fsize)?;
            if falign > max_align {
                max_align = falign;
            }
        }
        return Some(align_up(offset, max_align));
    }

    for e in nominals.enums.iter() {
        if e.name != ty {
            continue;
        }
        return type_size_bytes_rec(e.base, nominals, depth - 1);
    }

    None
}

fn array_size_bytes(ty_bytes: &[u8], nominals: &NominalDb, depth: u8) -> Option<u32> {
    if !ty_bytes.starts_with(b"Array(") || !ty_bytes.ends_with(b")") {
        return None;
    }
    let inner = &ty_bytes[b"Array(".len()..ty_bytes.len() - 1];
    let mut depth_paren = 0u32;
    for (i, &c) in inner.iter().enumerate() {
        match c {
            b'(' => depth_paren = depth_paren.wrapping_add(1),
            b')' => depth_paren = depth_paren.wrapping_sub(1),
            b',' if depth_paren == 0 => {
                let elem = TypeAtom::new(&inner[..i])?;
                let len = parse_u32_any(&inner[i + 1..])?;
                let elem_size = type_size_bytes_rec(elem, nominals, depth - 1)?;
                return len.checked_mul(elem_size);
            }
            _ => {}
        }
    }
    None
}

pub fn field_align(size: u32) -> u32 {
    if size >= 8 {
        8
    } else if size >= 4 {
        4
    } else if size >= 2 {
        2
    } else {
        1
    }
}

pub fn align_up(value: u32, align: u32) -> u32 {
    if align == 0 {
        return value;
    }
    let mask = align - 1;
    (value + mask) & !mask
}

pub fn chan_elem_type(ty: TypeAtom) -> Option<TypeAtom> {
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

pub fn parse_i64_token(token: &[u8]) -> Option<i64> {
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

pub fn apply_sig(
    stack: &mut [Value; 256],
    sp: &mut usize,
    entry: &WordEntry,
    span: Span,
    subtypes: &[SubtypeInfo],
) -> Result<(), TcError> {
    if entry.performs.contains(EffectSet::SUSPEND) && !check_no_scoped_live(stack, *sp) {
        return Err(TcError::ScopedLiveAtSuspend { span });
    }

    let sig = &entry.sig;
    let need = sig.in_len as usize;
    if *sp < need {
        return Err(TcError::SigStackUnderflow { span });
    }
    // Check types from top.
    for i in 0..need {
        let got = stack[*sp - need + i];
        let got = got.to_type_atom();
        if !type_compatible(got, sig.inputs[i], subtypes) {
            return Err(TcError::SigTypeMismatch { span });
        }
    }
    *sp -= need;
    for i in 0..(sig.out_len as usize) {
        push(stack, sp, Value::Plain(sig.outputs[i]))?;
    }
    Ok(())
}

pub fn write_sig(out: &mut impl Output, sig: &WordSig) {
    out.write(b"( ");
    for i in 0..(sig.in_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        write_type(out, sig.inputs[i], 4);
    }
    out.write(b" --");
    if sig.out_len > 0 {
        out.write(b" ");
    }
    for i in 0..(sig.out_len as usize) {
        if i != 0 {
            out.write(b" ");
        }
        write_type(out, sig.outputs[i], 4);
    }
    out.write(b" )");
}

fn write_type(out: &mut impl Output, ty: TypeAtom, depth: u8) {
    if depth == 0 {
        out.write(ty.as_bytes());
        return;
    }
    let bytes = ty.as_bytes();
    if bytes.starts_with(b"Chan(") && bytes.ends_with(b")") {
        let inner = &bytes[b"Chan(".len()..bytes.len() - 1];
        out.write(b"|");
        if let Some(atom) = TypeAtom::new(inner) {
            write_type(out, atom, depth - 1);
        } else {
            out.write(inner);
        }
        out.write(b"|");
        return;
    }
    if bytes.starts_with(b"Array(") && bytes.ends_with(b")") {
        let inner = &bytes[b"Array(".len()..bytes.len() - 1];
        let mut depth_paren = 0u32;
        for (i, &c) in inner.iter().enumerate() {
            match c {
                b'(' => depth_paren = depth_paren.wrapping_add(1),
                b')' => depth_paren = depth_paren.wrapping_sub(1),
                b',' if depth_paren == 0 => {
                    let elem = &inner[..i];
                    let len = &inner[i + 1..];
                    if let Some(atom) = TypeAtom::new(elem) {
                        write_type(out, atom, depth - 1);
                    } else {
                        out.write(elem);
                    }
                    out.write(b"'");
                    out.write(len);
                    return;
                }
                _ => {}
            }
        }
    }
    out.write(bytes);
}

pub fn write_stack(out: &mut dyn Output, stack: &[Value; 256], sp: usize) {
    for (i, v) in stack.iter().enumerate().take(sp) {
        if i != 0 {
            out.write(b" ");
        }
        match v {
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

