use frontend::{
    parse::{DeclAst, DeclKind, ModuleAst},
    span::Span,
};
use ir as lir;

pub fn fnv1a_u32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

pub fn mask_for_bits(bits: u16) -> u64 {
    if bits >= 64 {
        !0u64
    } else {
        (1u64 << bits) - 1
    }
}

pub fn prim_bits_signed(ty: &[u8]) -> Option<(u16, bool)> {
    if ty.starts_with(b"Chan(") {
        return Some((64, false));
    }
    let (bits, signed) = match ty {
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

pub fn prim_ty_bits_signed(w: &lir::Word, ty: lir::TypeId) -> Option<(u16, bool)> {
    let b = w.types.get(ty.0 as usize).map(|a| a.as_bytes())?;
    prim_bits_signed(b)
}

pub fn slice_span(src: &[u8], span: Span) -> &[u8] {
    &src[span.start..span.end]
}

pub fn is_exported(module: &ModuleAst, src: &[u8], name: &[u8]) -> bool {
    if !module.has_export_stmt {
        return true;
    }
    for span in module.exports.iter() {
        if slice_span(src, *span) == name {
            return true;
        }
    }
    false
}

pub fn line_col(src: &[u8], offset: usize) -> (u32, u32) {
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    let mut i = 0usize;
    let end = core::cmp::min(offset, src.len());
    while i < end {
        if src[i] == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        i += 1;
    }
    (line, col)
}

pub fn write_u32(out: &mut dyn frontend::parse::Output, mut v: u32) {
    let mut buf = [0u8; 10];
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

pub fn write_u64_hex(out: &mut dyn frontend::parse::Output, v: u64) {
    out.write(b"0x");
    let mut buf = [0u8; 16];
    for (i, byte) in buf.iter_mut().enumerate() {
        let shift = (15 - i) * 4;
        let nib = ((v >> shift) & 0xF) as u8;
        *byte = match nib {
            0..=9 => b'0' + nib,
            _ => b'a' + (nib - 10),
        };
    }
    let mut start = 0usize;
    while start + 1 < buf.len() && buf[start] == b'0' {
        start += 1;
    }
    out.write(&buf[start..]);
}

pub fn hex_digit(v: u8) -> u8 {
    match v {
        0..=9 => b'0' + v,
        _ => b'a' + (v - 10),
    }
}

pub fn find_word_decl<'a>(m: &'a ModuleAst, src: &[u8], name: &[u8]) -> Option<&'a DeclAst> {
    for d in m.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        if slice_span(src, d.name) == name {
            return Some(d);
        }
    }
    None
}

pub fn max_local_slot_ir(w: &lir::Word) -> Option<u16> {
    let mut max = None;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            let s = match op.kind {
                lir::OpKind::LocalSet { slot, .. } => Some(slot),
                lir::OpKind::LocalGet { slot, .. } => Some(slot),
                _ => None,
            };
            if let Some(s) = s {
                max = Some(match max {
                    Some(m) => core::cmp::max(m, s),
                    None => s,
                });
            }
        }
    }
    max
}

pub fn count_scoped_slices(w: &lir::Word) -> u32 {
    let mut count = 0u32;
    for b in w.blocks.iter() {
        for op in b.ops.iter() {
            if let lir::OpKind::ScopedEnter { ty, .. } = op.kind {
                let name = w.types.get(ty.0 as usize).map(|a| a.as_bytes()).unwrap_or(b"");
                if name.starts_with(b"Slice(") || name.starts_with(b"SliceMut(") {
                    count = count.wrapping_add(1);
                }
            }
        }
    }
    count
}

pub fn locals_bytes_ir(slots: u32) -> u32 {
    if slots == 0 {
        return 0;
    }
    let mut bytes = slots * 8;
    if !bytes.is_multiple_of(16) {
        bytes += 8;
    }
    bytes
}

pub fn type_size_bytes(w: &lir::Word, ty: lir::TypeId) -> Option<u32> {
    let size = *w.type_sizes.get(ty.0 as usize)?;
    if size == 0 {
        None
    } else {
        Some(size)
    }
}
