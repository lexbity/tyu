use frontend::parse::Output;
use frontend::span::Span;

pub fn write_u32(out: &mut dyn Output, v: u32) {
    let mut buf = [0u8; 12];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    } else {
        let mut u = v;
        while u > 0 && n < buf.len() {
            buf[n] = b'0' + (u % 10) as u8;
            n += 1;
            u /= 10;
        }
        buf[..n].reverse();
    }
    out.write(&buf[..n]);
}

pub fn write_hex(out: &mut dyn Output, v: u64) {
    let mut buf = [0u8; 20];
    buf[0] = b'0';
    buf[1] = b'x';
    let mut n = 2usize;
    for i in (0..64).step_by(4).rev() {
        let nib = ((v >> i) & 0xf) as u8;
        if n > 2 || nib != 0 || i == 0 {
            buf[n] = if nib < 10 { b'0' + nib } else { b'a' + nib - 10 };
            n += 1;
        }
    }
    if n == 2 {
        buf[n] = b'0';
        n += 1;
    }
    out.write(&buf[..n]);
}

pub fn write_sym_label(out: &mut dyn Output, name: &[u8]) {
    let hash = fnv1a_u64(name);
    out.write(b"w_");
    for i in (0..64).step_by(4).rev() {
        let nib = ((hash >> i) & 0xf) as u8;
        out.write(&[if nib < 10 { b'0' + nib } else { b'a' + nib - 10 }]);
    }
}

pub fn fnv1a_u64(name: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in name {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn slice_span<'a>(src: &'a [u8], span: Span) -> &'a [u8] {
    let start = span.start as usize;
    let end = span.end as usize;
    if start < end && end <= src.len() {
        &src[start..end]
    } else {
        &[]
    }
}
