use codegen_core::{Feature, FeatureSet};
use frontend::parse::Output;
use frontend::span::Span;
use hosted::{diag, errno::Errno, fs, io};
use ir as lir;

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

// ---------------------------------------------------------------------------
// Feature gate diagnostics (E61xx)
// ---------------------------------------------------------------------------

/// What feature a particular `OpKind` requires, if any.
///
/// Returns `None` for ops that are always available (ungated).
pub fn op_requires_feature(kind: lir::OpKind) -> Option<Feature> {
    match kind {
        lir::OpKind::TaskSpawn { .. } => Some(Feature::Concurrency),
        // Module-loading ops will gate on Feature::ModuleLoading when they
        // are added to the IR (e.g. OpKind::ModuleLoad { .. }).
        // The op_requires_feature match arm is reserved here so the gate
        // infrastructure is ready; no such ops exist yet.
        _ => None,
    }
}

/// Emit `error[E6101]: feature `xxx` is disabled` with a source span.
///
/// Renders:
/// ```text
/// error[E6101]: feature `concurrency` is disabled
///   --> path.mod:12:3
///    |
/// 12 |   task spawn worker;
///    |   ^^^^^^^^^^^^^^^^^^ requires feature `concurrency`
/// ```
pub fn emit_gate_error(path: &[u8], src: &[u8], feature: Feature, span: Span) {
    let (line, col) = line_col(src, span.start);
    let feat_name = feature.as_str();

    // error[E6101]
    let _ = io::stderr(b"error[E6101]: feature `");
    let _ = io::stderr(feat_name.as_bytes());
    let _ = io::stderr(b"` is disabled\n");

    //   --> path:line:col
    let _ = io::stderr(b"  --> ");
    let _ = io::stderr(path);
    let _ = io::stderr(b":");
    write_u32_stderr(line);
    let _ = io::stderr(b":");
    write_u32_stderr(col);

    // Source line with caret
    let _ = io::stderr(b"\n   |\n");
    // Find the source line containing the span
    let mut line_start = span.start;
    while line_start > 0 && src[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let mut line_end = span.end;
    while line_end < src.len() && src[line_end] != b'\n' {
        line_end += 1;
    }
    // Print the source line
    let _ = io::stderr(b" ");
    write_u32_stderr(line);
    let _ = io::stderr(b" | ");
    let _ = io::stderr(&src[line_start..line_end]);
    let _ = io::stderr(b"\n");

    // Caret line: spaces then ^^^ under the span
    let caret_col = span.start - line_start;
    let caret_width = core::cmp::max(1, span.end - span.start);
    let _ = io::stderr(b"   | ");
    for _ in 0..caret_col {
        let _ = io::stderr(b" ");
    }
    for _ in 0..caret_width {
        let _ = io::stderr(b"^");
    }
    let _ = io::stderr(b" requires feature `");
    let _ = io::stderr(feat_name.as_bytes());
    let _ = io::stderr(b"`\n");
}

/// Check every `Op` in `word` against the feature gate.
/// Returns `true` if any gated op was found (and its diagnostics emitted).
pub fn check_word_for_gate(
    word: &lir::Word,
    feature_set: FeatureSet,
    path: &[u8],
    src: &[u8],
) -> bool {
    let mut hit = false;
    for bi in 0..word.blocks.len() {
        let block = match word.blocks.get(bi) {
            Some(b) => b,
            None => break,
        };
        for oi in 0..block.ops.len() {
            let op = match block.ops.get(oi) {
                Some(o) => o,
                None => break,
            };
            if let Some(required) = op_requires_feature(op.kind) {
                if !feature_set.contains(required) {
                    emit_gate_error(path, src, required, op.span);
                    hit = true;
                }
            }
        }
    }
    hit
}
