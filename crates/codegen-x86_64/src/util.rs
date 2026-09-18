use frontend::{
    parse::{DeclAst, DeclKind, ModuleAst},
    span::Span,
};
use ir as lir;

pub fn fnv1a_u64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
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

pub fn prim_ty(w: &lir::Word, ty: lir::TypeId) -> Option<lir::Prim> {
    let b = w.types.get(ty.0 as usize).map(|a| a.as_bytes())?;
    lir::Prim::from_type_name(b)
}

pub fn prim_ty_bits_signed(w: &lir::Word, ty: lir::TypeId) -> Option<(u16, bool)> {
    prim_ty(w, ty).map(|prim| prim.bits_signed(64))
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

/// Find a `resource` declaration by name (BUG-004: resources are addressable
/// through `&`/`&!`; the backend must emit the resource symbol address).
pub fn find_resource_decl<'a>(
    m: &'a ModuleAst,
    src: &[u8],
    name: &[u8],
) -> Option<&'a DeclAst> {
    for d in m.decls.iter() {
        if d.kind != DeclKind::Resource {
            continue;
        }
        if slice_span(src, d.name) == name {
            return Some(d);
        }
    }
    None
}

/// Names of every `resource` declared in the module.
pub fn resource_decl_names<'a>(
    m: &'a ModuleAst,
    src: &'a [u8],
) -> impl Iterator<Item = &'a [u8]> {
    m.decls.iter().filter_map(move |d| {
        if d.kind != DeclKind::Resource {
            return None;
        }
        Some(slice_span(src, d.name))
    })
}

/// Emit the resource symbol label `r_<fnv1a(module) ⊙ name>` — identical to
/// the ARM backend's scheme (`r_` + hashed module/name) so resource symbols
/// are consistent across backends.
pub fn write_res_label(
    out: &mut dyn frontend::parse::Output,
    module_name: &[u8],
    resource_name: &[u8],
) {
    let mut hash = fnv1a_u64(module_name);
    hash ^= 0xff;
    hash = hash.wrapping_mul(1099511628211);
    for &b in resource_name {
        hash ^= b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    out.write(b"r_");
    for i in (0..64).step_by(4).rev() {
        let nib = ((hash >> i) & 0xf) as u8;
        out.write(&[hex_digit(nib)]);
    }
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
                let name = w
                    .types
                    .get(ty.0 as usize)
                    .map(|a| a.as_bytes())
                    .unwrap_or(b"");
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
