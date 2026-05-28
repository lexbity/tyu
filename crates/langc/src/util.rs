use frontend::parse::Output;
use frontend::span::Span;
use hosted::{diag, errno::Errno, fs, io};

pub struct Stdout;

impl Output for Stdout {
    fn write(&mut self, bytes: &[u8]) {
        let _ = io::stdout(bytes);
    }
}

pub struct MemOut {
    pub buf: fs::ByteBuf,
    pub err: Option<Errno>,
}

impl MemOut {
    pub fn new() -> Result<Self, Errno> {
        Ok(Self {
            buf: fs::ByteBuf::new()?,
            err: None,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        self.buf.as_slice()
    }
}

impl Output for MemOut {
    fn write(&mut self, bytes: &[u8]) {
        if self.err.is_some() {
            return;
        }
        if let Err(e) = self.buf.push_slice(bytes) {
            self.err = Some(e);
        }
    }
}

pub fn emit_parse_error(path: &[u8], src: &[u8], code: u32, offset: usize) {
    let (line, col) = line_col(src, offset);
    let _ = io::stderr(path);
    let _ = io::stderr(b":");
    write_u32_stderr(line);
    let _ = io::stderr(b":");
    write_u32_stderr(col);
    let _ = io::stderr(b" ");
    let _ = diag::error_simple(code, b"parse error");
}

pub fn write_u32_stderr(mut v: u32) {
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
    let _ = io::stderr(&buf[..n]);
}

pub fn slice_span(src: &[u8], span: Span) -> &[u8] {
    &src[span.start..span.end]
}

pub fn try_load_module_file(
    search_dirs: &[&[u8]],
    module: &[u8],
    ext: &[u8],
) -> Option<hosted::fs::ByteBuf> {
    let mut path_buf = [0u8; 512];
    for d in search_dirs {
        let path = join_path(&mut path_buf, d, module, ext)?;
        if let Ok(b) = fs::read_file(path) {
            return Some(b);
        }
    }
    None
}

pub fn join_path<'a>(buf: &'a mut [u8], dir: &[u8], module: &[u8], ext: &[u8]) -> Option<&'a [u8]> {
    let dir = if dir == b"." { b"" } else { dir };
    let sep: &[u8] = if dir.is_empty() || dir.ends_with(b"/") {
        b""
    } else {
        b"/"
    };
    let need = dir.len() + sep.len() + module.len() + ext.len();
    if need > buf.len() {
        return None;
    }
    let mut i = 0usize;
    buf[i..i + dir.len()].copy_from_slice(dir);
    i += dir.len();
    buf[i..i + sep.len()].copy_from_slice(sep);
    i += sep.len();
    buf[i..i + module.len()].copy_from_slice(module);
    i += module.len();
    buf[i..i + ext.len()].copy_from_slice(ext);
    i += ext.len();
    Some(&buf[..i])
}

pub fn split_dir<'a>(path: &[u8], out: &'a mut [u8]) -> &'a [u8] {
    let mut last = None;
    for (i, &b) in path.iter().enumerate() {
        if b == b'/' {
            last = Some(i);
        }
    }
    let Some(idx) = last else {
        out[0] = b'.';
        return &out[..1];
    };
    if idx == 0 {
        out[0] = b'/';
        return &out[..1];
    }
    if idx > out.len() {
        out[0] = b'.';
        return &out[..1];
    }
    out[..idx].copy_from_slice(&path[..idx]);
    &out[..idx]
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

