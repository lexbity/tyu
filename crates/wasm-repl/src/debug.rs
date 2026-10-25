//! The debug data substrate (PLAN-WASM-2 W1, DEC-20/DEC-24/DEC-29): read-only
//! snapshot projections of the session/engine state — the ONLY source every
//! debug surface (native `repl-cli debug-dump`, the wasm ABI, the browser
//! console) renders from (I-8, I-11, R-E). The JS layer and the CLI MUST NOT
//! synthesize, infer, or reconstruct debug facts; every fact here is derived
//! from engine state or from the exact submitted buffer (the I-9 anchor).
//!
//! Zero-dependency, `no_std`, no fallbacks: unknown inputs produce structured
//! empty/NotFound values, never guesses and never panics.

use alloc::vec::Vec;
use frontend::lex::Lexer;
use frontend::span::Span;
use frontend::token::TokenKind;

use crate::compile::{Compiled, SourceTable};
use crate::engine::ExecState;
use crate::fuel::Fuel;
use crate::heap::RegionHeap;
use crate::session::StackCell;
use crate::sysroot::SysrootTable;
use crate::trace::{Event, Trace};
use crate::value::{format_hex, Slot};

/// One call frame, innermost-first (gdb `bt` order). Derived only from engine
/// state (I-8). Payloads are complete up to the engine caps (§Q9's data-side
/// rule) — render-side caps are the UI's job (DEC-25).
#[derive(Clone, Debug, PartialEq)]
pub struct FrameCell {
    /// 0 = innermost frame.
    pub index: u32,
    pub word: Vec<u8>,
    /// The declared signature, `( in -- out )`.
    pub sig: Vec<u8>,
    /// The call site (I-9 encoding, into the current session buffer). The
    /// entry frame has `Span::UNKNOWN` — nothing called it.
    pub call_span: Span,
    pub locals: Vec<StackCell>,
}

/// The normalized trace of the LAST submission (DEC-20), projected from the
/// retained [`Trace`]. Journal-internal effects (Store/RegionCreate/…) are
/// NOT exposed — they are transaction machinery, not student-facing facts.
#[derive(Clone, Debug, PartialEq)]
pub enum TraceEventView {
    /// Diagnostic-channel bytes: a range of the session's total output.
    Output { start: u32, len: u32 },
    /// A peripheral access (aperture, offset, width, value, read/write).
    Periph {
        ap: u16,
        off: u32,
        width: u8,
        val: u64,
        is_write: bool,
    },
    /// A word call (call pane / post-mortem backtrace spine).
    Call { word: Vec<u8>, depth: u16 },
    /// A trap: frozen numeric + span + the §Q6 detail when it is a contract
    /// failure (absent otherwise — consumers MUST treat absent as "not a
    /// contract failure", never guess).
    Trap {
        code: u32,
        span: Span,
        detail: Option<crate::engine::TrapDetail>,
    },
    /// A device-level failure (E9152 family).
    Device { code: u32 },
    /// An engine-internal fault (E9158 / E9156 backstop) — always a bug.
    Internal { code: u32 },
    /// Fuel exhausted (the phrase is suspended).
    Fuel,
    /// Natural completion or `testio.exit`.
    Done { exit: i64 },
}

/// A word's origin (DEC-29): session words resolve to the buffer; sysroot
/// words resolve into the blob's file map.
#[derive(Clone, Debug, PartialEq)]
pub enum WordOrigin {
    /// Defined in the session buffer at `def_span` (`&src[span]` is the
    /// definition's name token — the I-9 anchor).
    Session { def_span: Span },
    /// Defined by a sysroot source file.
    Sysroot { path: Vec<u8> },
}

/// One dictionary word (completions, `?word` introspection, the dictionary
/// count in the status bar).
#[derive(Clone, Debug, PartialEq)]
pub struct WordInfo {
    pub name: Vec<u8>,
    /// `( in -- out )` rendered from the IR `Sig`.
    pub sig: Vec<u8>,
    /// The declared effect set, rendered (`-` when pure).
    pub effects: Vec<u8>,
    pub origin: WordOrigin,
}

/// View-source resolution (DEC-29): sysroot from the blob; session words
/// resolve to the buffer; unknown names are a structured not-found — never a
/// guess.
#[derive(Clone, Debug, PartialEq)]
pub enum WordSource {
    /// A session word: show the definition's range of the editor buffer.
    Session { def_span: Span },
    /// A sysroot word: the defining file's path and full source.
    Sysroot { path: Vec<u8>, source: Vec<u8> },
    NotFound,
}

/// One region-table slot (the heap inspector's row). `kind` is 0 (data) —
/// the as-built region table carries no kind tag (the W1 reconciliation:
/// the plan's `kind: u8` has no IR/heap counterpart yet; the field stays so
/// the ABI shape is stable when one lands).
#[derive(Clone, Debug, PartialEq)]
pub struct RegionInfo {
    pub id: u32,
    pub off: u32,
    pub len: u32,
    pub cursor: u32,
    pub kind: u8,
    pub alive: bool,
}

/// The session-level status summary (the status bar's fields, §7.4 — all
/// from ABI data, no JS-side inference).
#[derive(Clone, Debug, PartialEq)]
pub struct SessionStatus {
    pub stack_depth: usize,
    pub word_count: usize,
    pub fuel_ops: u64,
}

/// The live frames of an engine state, innermost-first (gdb order). After a
/// trap the state has rolled back — the POST-MORTEM backtrace is built from
/// `trace_events` (§Q8); this projection serves the LIVE case only
/// (suspension / mid-`step`).
pub fn frames(compiled: &Compiled, state: &ExecState) -> Vec<FrameCell> {
    let mut out = Vec::new();
    let n = state.frames.len();
    for (pos, frame) in state.frames.iter().enumerate().rev() {
        let index = (n - 1 - pos) as u32;
        let word = &compiled.module.words[frame.word];
        // The call site is the op BEFORE the frame's return cursor (the
        // `Call` that pushed this frame); the entry frame was not called.
        let call_span = match frame.ret {
            Some(ret) => compiled
                .module
                .words
                .get(ret.word)
                .and_then(|w| w.blocks.get(ret.block))
                .and_then(|b| ret.op.checked_sub(1).and_then(|i| b.ops.get(i)))
                .map(|op| op.span)
                .unwrap_or(Span::UNKNOWN),
            None => Span::UNKNOWN,
        };
        out.push(FrameCell {
            index,
            word: word.name.as_bytes().to_vec(),
            sig: crate::engine::sig_text_of(word),
            call_span,
            locals: frame.locals.iter().map(|&s| cell_of(word, s)).collect(),
        });
    }
    out
}

/// The last submission's normalized trace, as [`TraceEventView`]s.
pub fn trace_events(trace: &Trace) -> Vec<TraceEventView> {
    trace
        .events()
        .iter()
        .map(trace_view_of)
        .collect()
}

fn trace_view_of(e: &Event) -> TraceEventView {
    match e {
        Event::Output { start, len } => TraceEventView::Output {
            start: *start,
            len: *len,
        },
        Event::Periph {
            aperture,
            off,
            width,
            val,
            is_write,
        } => TraceEventView::Periph {
            ap: *aperture,
            off: *off,
            width: *width,
            val: *val,
            is_write: *is_write,
        },
        Event::Call { word, depth } => TraceEventView::Call {
            word: word.as_bytes().to_vec(),
            depth: *depth,
        },
        Event::Trap { code, span, detail } => TraceEventView::Trap {
            code: ir::trap_code_u32(*code),
            span: *span,
            detail: detail.clone(),
        },
        Event::Device { code } => TraceEventView::Device { code: *code },
        Event::Internal { code } => TraceEventView::Internal { code: *code },
        Event::Fuel => TraceEventView::Fuel,
        Event::Done { exit } => TraceEventView::Done { exit: *exit },
    }
}

/// The session's dictionary: module words (session-defined — imports
/// contribute signatures only, never bodies) with their definition spans
/// resolved against the session buffer.
pub fn words(compiled: &Compiled, source: &[u8]) -> Vec<WordInfo> {
    compiled
        .module
        .words
        .iter()
        // The synthetic `repl-phrase` word is session machinery (the §7.2
        // phrase wrapper), not a student-facing dictionary entry.
        .filter(|w| w.name.as_bytes() != b"repl-phrase")
        .map(|w| WordInfo {
            name: w.name.as_bytes().to_vec(),
            sig: crate::engine::sig_text_of(w),
            effects: effects_text(w.performs),
            origin: WordOrigin::Session {
                def_span: def_span(source, w.name.as_bytes()).unwrap_or(Span::UNKNOWN),
            },
        })
        .collect()
}

/// The sysroot file map `word_source` resolves against (DEC-29): the wasm
/// boundary backs it with the `tyu-sysroot/1` blob; host/CI harnesses back
/// it with the same directory map the compiler searched (§Q12). Sorted by
/// path; `(path, content)` pairs.
pub trait SysrootFiles {
    fn files(&self) -> Vec<(Vec<u8>, Vec<u8>)>;
}

impl SysrootFiles for SysrootTable {
    fn files(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.paths()
            .filter_map(|p| self.read(p).map(|c| (p.to_vec(), c.to_vec())))
            .collect()
    }
}

/// View-source resolution (DEC-29): a session word shows its buffer range; a
/// sysroot word shows the defining blob file; anything else is `NotFound`.
/// `sysroot` may be `None` (a session with no blob — bare sessions).
pub fn word_source(
    compiled: Option<&Compiled>,
    source: &[u8],
    sysroot: Option<&dyn SysrootFiles>,
    name: &[u8],
) -> WordSource {
    if let Some(c) = compiled {
        if c.module.words.iter().any(|w| w.name.as_bytes() == name) {
            return WordSource::Session {
                def_span: def_span(source, name).unwrap_or(Span::UNKNOWN),
            };
        }
    }
    if let Some(table) = sysroot {
        for (path, content) in table.files() {
            if defines_word(&content, name) {
                return WordSource::Sysroot { path, source: content };
            }
        }
    }
    WordSource::NotFound
}

/// The region table (the heap inspector's rows), handle order.
pub fn regions(heap: &RegionHeap) -> Vec<RegionInfo> {
    let mut out = Vec::new();
    heap.for_each_region(|handle, base, size, off, alive| {
        out.push(RegionInfo {
            id: handle as u32,
            off: base,
            len: size,
            cursor: off,
            kind: 0,
            alive,
        });
    });
    out
}

/// The status summary (§7.4): stack depth, dictionary size, fuel budget.
pub fn status(stack_depth: usize, compiled: Option<&Compiled>, fuel: Fuel) -> SessionStatus {
    SessionStatus {
        stack_depth,
        word_count: compiled
            .map(|c| {
                c.module
                    .words
                    .iter()
                    .filter(|w| w.name.as_bytes() != b"repl-phrase")
                    .count()
            })
            .unwrap_or(0),
        fuel_ops: fuel.remaining(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cell_of(word: &ir::Word, s: Slot) -> StackCell {
    StackCell {
        ty_name: crate::engine::type_name_of(word, s.ty),
        value_hex: format_hex(s.val.as_u64()),
    }
}

/// The declared effect set as text (`-` = pure; bit names otherwise).
fn effects_text(performs: ir::EffectSet) -> Vec<u8> {
    const NAMES: [(&[u8], u16); 5] = [
        (b"suspend", ir::EffectSet::SUSPEND),
        (b"interrupt", ir::EffectSet::INTERRUPT),
        (b"diverge", ir::EffectSet::DIVERGE),
        (b"mmio", ir::EffectSet::MMIO),
        (b"alloc", ir::EffectSet::ALLOC),
    ];
    if performs.is_empty() {
        return b"-".to_vec();
    }
    let mut out = Vec::new();
    for (name, bit) in NAMES {
        if performs.contains(bit) {
            if !out.is_empty() {
                out.push(b'+');
            }
            out.extend_from_slice(name);
        }
    }
    out
}

/// The span of `name`'s definition in a source buffer: the `: name` opener's
/// name token (the session's word-definition convention, §7.2). `None` when
/// the buffer does not define it (never a guess).
fn def_span(source: &[u8], name: &[u8]) -> Option<Span> {
    let mut lex = Lexer::new(source);
    let mut prev_colon = false;
    loop {
        let t = lex.next();
        if t.kind == TokenKind::Eof {
            return None;
        }
        if prev_colon
            && t.kind == TokenKind::Ident
            && &source[t.span.start..t.span.end] == name
        {
            return Some(t.span);
        }
        prev_colon = t.kind == TokenKind::PunctColon;
    }
}

/// Does a sysroot source file define `name` (the same `: name` convention)?
fn defines_word(source: &[u8], name: &[u8]) -> bool {
    def_span(source, name).is_some()
}

// ---------------------------------------------------------------------------
// The debug dump (PLAN-WASM-2 FR-27/G57): per submission, the exact snapshot
// payloads the wasm ABI exports, as one deterministic JSON line. The encoder
// lives HERE — the native CLI (`repl-cli debug-dump`) and any test through
// the core API emit the same bytes (R-E / I-11: one core, both surfaces,
// one renderer per fact). Hand-rolled deterministic JSON; std not required.
// ---------------------------------------------------------------------------

use alloc::format;
use alloc::string::{String, ToString};

fn json_escape(b: &[u8]) -> String {
    let mut s = String::from("\"");
    for &c in b {
        match c {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7e => s.push(c as char),
            _ => s.push_str(&format!("\\u{:04x}", c)),
        }
    }
    s.push('"');
    s
}

fn json_cell(c: &StackCell) -> String {
    format!(
        "{{\"ty_name\":{},\"value_hex\":{}}}",
        json_escape(&c.ty_name),
        json_escape(&c.value_hex)
    )
}

fn json_span(sp: Span) -> String {
    format!(
        "{{\"start\":{},\"len\":{}}}",
        sp.start,
        sp.end.saturating_sub(sp.start)
    )
}

fn json_detail(d: &crate::engine::TrapDetail) -> String {
    let actual: Vec<String> = d.actual.iter().map(json_cell).collect();
    format!(
        "{{\"kind\":{},\"word\":{},\"declared\":{},\"actual\":[{}]}}",
        json_escape(d.kind.name()),
        json_escape(&d.word),
        json_escape(&d.declared),
        actual.join(",")
    )
}

fn json_event_doc(e: &TraceEventView, doc: &dyn Fn(Span) -> Span) -> String {
    match e {
        TraceEventView::Output { start, len } => {
            format!("{{\"kind\":\"output\",\"start\":{start},\"len\":{len}}}")
        }
        TraceEventView::Periph {
            ap,
            off,
            width,
            val,
            is_write,
        } => format!(
            "{{\"kind\":\"periph\",\"ap\":{ap},\"off\":{off},\"width\":{width},\"val\":{val:#x},\"is_write\":{is_write}}}"
        ),
        TraceEventView::Call { word, depth } => format!(
            "{{\"kind\":\"call\",\"word\":{},\"depth\":{}}}",
            json_escape(word),
            depth
        ),
        TraceEventView::Trap { code, span, detail } => {
            let d = detail
                .as_ref()
                .map(json_detail)
                .unwrap_or_else(|| "null".to_string());
            format!(
                "{{\"kind\":\"trap\",\"code\":{code},\"span\":{},\"detail\":{d}}}",
                json_span(doc(*span))
            )
        }
        TraceEventView::Device { code } => format!("{{\"kind\":\"device\",\"code\":{code}}}"),
        TraceEventView::Internal { code } => format!("{{\"kind\":\"internal\",\"code\":{code}}}"),
        TraceEventView::Fuel => "{\"kind\":\"fuel\"}".to_string(),
        TraceEventView::Done { exit } => format!("{{\"kind\":\"done\",\"exit\":{exit}}}"),
    }
}

fn json_word(w: &WordInfo) -> String {
    let origin = match &w.origin {
        WordOrigin::Session { def_span } => {
            format!("{{\"origin\":\"session\",\"def_span\":{}}}", json_span(*def_span))
        }
        WordOrigin::Sysroot { path } => {
            format!("{{\"origin\":\"sysroot\",\"path\":{}}}", json_escape(path))
        }
    };
    format!(
        "{{\"name\":{},\"sig\":{},\"effects\":{},\"origin\":{}}}",
        json_escape(&w.name),
        json_escape(&w.sig),
        json_escape(&w.effects),
        origin
    )
}

fn json_region(r: &RegionInfo) -> String {
    format!(
        "{{\"id\":{},\"off\":{:#x},\"len\":{:#x},\"cursor\":{:#x},\"kind\":{},\"alive\":{}}}",
        r.id, r.off, r.len, r.cursor, r.kind, r.alive
    )
}

/// One submission's snapshot line (§5.2): frames / trace events / words /
/// regions / status — derived ONLY from the session's own accessors (I-8),
/// the same data the wasm ABI wraps. The G57(a) parity test compares this
/// byte-for-byte against the CLI's output.
pub fn dump_submission(session: &crate::session::ReplSession, submit_index: usize, source: &[u8]) -> String {
    // I-9: every span in the dump is DOC-relative — the same mapping the
    // wasm ABI applies (I-11: one mapper, both surfaces; unmappable
    // machinery spans collapse to UNKNOWN, never an anchor offset).
    let doc = |span: Span| -> Span {
        session.doc_span(span).unwrap_or(Span::UNKNOWN)
    };
    let compiled = session.compiled();
    let frames: Vec<String> = match session.suspended_state().zip(compiled) {
        Some((st, c)) => frames(c, st)
            .iter()
            .map(|f| {
                format!(
                    "{{\"index\":{},\"word\":{},\"sig\":{},\"call_span\":{},\"locals\":[{}]}}",
                    f.index,
                    json_escape(&f.word),
                    json_escape(&f.sig),
                    json_span(doc(f.call_span)),
                    f.locals.iter().map(json_cell).collect::<Vec<_>>().join(",")
                )
            })
            .collect(),
        None => Vec::new(),
    };
    let events: Vec<String> = match session.last_trace() {
        Some(t) => trace_events(t)
            .iter()
            .map(|e| json_event_doc(e, &doc))
            .collect(),
        None => Vec::new(),
    };
    let words: Vec<String> = match compiled {
        Some(c) => words(c, session.source())
            .iter()
            .map(|w| {
                let mut w = w.clone();
                if let WordOrigin::Session { def_span } = &mut w.origin {
                    *def_span = doc(*def_span);
                }
                json_word(&w)
            })
            .collect(),
        None => Vec::new(),
    };
    let regions: Vec<String> = regions(session.heap())
        .iter()
        .filter(|r| r.alive)
        .map(json_region)
        .collect();
    let st = status(session.stack_cells().len(), compiled, session.fuel());
    let status_text = match session.last_status() {
        None => alloc::borrow::ToOwned::to_owned("fresh"),
        Some(s) => s.word(),
    };
    format!(
        "{{\"submit\":{},\"source\":{},\"status\":{},\"stack_depth\":{},\"words_count\":{},\"fuel\":{},\"frames\":[{}],\"trace_events\":[{}],\"words\":[{}],\"regions\":[{}]}}",
        submit_index,
        json_escape(source),
        json_escape(status_text.as_bytes()),
        st.stack_depth,
        st.word_count,
        st.fuel_ops,
        frames.join(","),
        events.join(","),
        words.join(","),
        regions.join(","),
    )
}
