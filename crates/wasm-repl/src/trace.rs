//! The `tyu.trace/1` event trace (PLAN-WASM-1 §7.4): event-oriented, so a
//! 100 M-op phrase never becomes 100 M records. The trace is **derived** from
//! the phrase's undo journal ([`crate::journal::Effect`]'s trace-only
//! variants) plus the run outcome — the journal IS the trace skeleton; this
//! module is its projection, not a second record.
//!
//! Goldens compare *normalized* traces — output structure, call sequences,
//! trap codes/spans, exit codes — never timings, never fuel-internal
//! counters. Version field first; consumers reject unknown versions
//! fail-closed.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use ir::{Span, TrapCode};

use crate::engine::Outcome;
use crate::journal::{Effect, Journal};

/// The trace schema version (fail-closed: consumers reject unknown versions).
pub const TRACE_SCHEMA: &str = "tyu.trace/1";
/// The header record's version number (FR-35): the first record of every
/// `tyu.trace/1` output.
pub const TRACE_VERSION: u32 = 1;

/// The header record (FR-35): `{v, spec, source_hash, fuel}` — target
/// identity, the SHA-256 of the exact submitted buffer (the I-9 span anchor),
/// and the phrase's op budget. Fuel checkpoints were considered and DROPPED
/// (oracle machinery, PLAN-WASM-2 §14 item 8) — the header is the only
/// fuel-facing record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceHeader {
    pub v: u32,
    pub spec: String,
    /// Full hex SHA-256 of the submitted source buffer.
    pub source_hash: String,
    /// The op budget the phrase ran under (remaining units at entry).
    pub fuel: u64,
}

impl TraceHeader {
    /// The normalized header text (the first field of `normalize`'s output):
    /// `v1 spec=<triple> src=<sha256> fuel=<n>`.
    pub fn normalize(&self) -> String {
        format!(
            "v{} spec={} src={} fuel={}",
            self.v, self.spec, self.source_hash, self.fuel
        )
    }
}

/// One derived trace event.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Diagnostic-channel bytes for the range `[start..start+len)` of the
    /// phrase output.
    Output { start: u32, len: u32 },
    /// A `Call` with the depth before the call (call pane).
    Call { word: String, depth: u16 },
    /// A peripheral access (P4): aperture, register offset, width, value,
    /// read/write.
    Periph {
        aperture: u16,
        off: u32,
        width: u8,
        val: u64,
        is_write: bool,
    },
    /// A device-level failure (P4, E9152-family) — the run ended on a
    /// peripheral diagnostic rather than a trap.
    Device { code: u32 },
    /// A trap with its frozen code + the faulting op's span. Contract
    /// failures (trap 20) carry the §Q6/DEC-22 detail (which word declared
    /// what, actual stack); every other trap's detail is absent, and
    /// consumers MUST treat absent detail as "not a contract failure".
    Trap {
        code: TrapCode,
        span: Span,
        detail: Option<crate::engine::TrapDetail>,
    },
    /// Fuel exhausted (E9150 surface).
    Fuel,
    /// An engine-internal fault (E9158, or E9156's runtime backstop) — the
    /// REPL-layer code is the event payload (never a frozen trap numeric).
    Internal { code: u32 },
    /// The run's exit (natural completion = 0; `testio.exit` passes its code).
    Done { exit: i64 },
}

/// A phrase trace, derived from the journal and the outcome. The header
/// (FR-35) is the first record; consumers reject unknown versions fail-closed.
#[derive(Clone, Debug)]
pub struct Trace {
    pub header: TraceHeader,
    events: Vec<Event>,
}

impl Trace {
    pub fn new(header: TraceHeader) -> Trace {
        Trace {
            header,
            events: Vec::new(),
        }
    }

    /// Project a phrase's observable trace: the version+header record, then
    /// journal `Output`/`Call` events plus the run outcome's trap/fuel/done
    /// tail. The journal is *walked*, not cloned (D-W4: the full-journal
    /// clone for trace derivation is gone — this is a view).
    pub fn from_phrase(header: TraceHeader, journal_effects: &Journal, outcome: &Outcome) -> Trace {
        let mut t = Trace::new(header);
        for e in journal_effects.iter() {
            match e {
                Effect::Output { start, len } => t.events.push(Event::Output {
                    start: *start,
                    len: *len,
                }),
                Effect::Call { word, depth_before } => t.events.push(Event::Call {
                    word: String::from_utf8_lossy(word).into_owned(),
                    depth: *depth_before,
                }),
                Effect::Periph {
                    aperture,
                    off,
                    width,
                    val,
                    is_write,
                } => t.events.push(Event::Periph {
                    aperture: *aperture,
                    off: *off,
                    width: *width,
                    val: *val,
                    is_write: *is_write,
                }),
                _ => {}
            }
        }
        match outcome {
            Outcome::Ok { .. } => t.events.push(Event::Done { exit: 0 }),
            Outcome::Exited { code, .. } => t.events.push(Event::Done { exit: *code }),
            Outcome::Trap {
                code, span, detail, ..
            } => t.events.push(Event::Trap {
                code: *code,
                span: *span,
                detail: detail.clone(),
            }),
            Outcome::Fuel { .. } => t.events.push(Event::Fuel),
            Outcome::Device { code, .. } => t.events.push(Event::Device { code: *code }),
            Outcome::Internal { fault, .. } => t.events.push(Event::Internal { code: fault.code }),
        }
        t
    }

    /// The normalized comparison text (golden-comparable, §7.4): the FR-35
    /// header record first, then event structure only — no timings, no
    /// fuel-internal counters. Call events above depth 32 are elided with a
    /// single `…` marker (the §7.4 bound — a >32-deep tail renders as
    /// `C{…}`, not per-frame noise). A contract trap's detail renders as a
    /// bracketed suffix (`[sig-mismatch word]`) — the only golden-visible
    /// delta the §Q6/DEC-22 extension admits.
    pub fn normalize(&self) -> String {
        let mut s = String::new();
        s.push_str(&self.header.normalize());
        s.push('|');
        let mut deep_calls = 0u32;
        let mut seen_deep = false;
        for e in &self.events {
            match e {
                Event::Output { len, .. } => s.push_str(&format!("O{len}")),
                Event::Call { word, depth } => {
                    if *depth > 32 {
                        seen_deep = true;
                        deep_calls += 1;
                    } else {
                        s.push_str(&format!("C{{{word}:{depth}}}"));
                    }
                }
                Event::Trap { code, detail, .. } => {
                    s.push_str(&format!("T{:?}", code));
                    if let Some(d) = detail {
                        s.push_str(&format!(
                            "[{} {}]",
                            String::from_utf8_lossy(d.kind.name()),
                            String::from_utf8_lossy(&d.word)
                        ));
                    }
                }
                Event::Periph {
                    aperture,
                    off,
                    width,
                    val,
                    is_write,
                } => {
                    s.push_str(&format!(
                        "P{}{:x}:{:x}:{}:{:x}",
                        if *is_write { 'W' } else { 'R' },
                        aperture,
                        off,
                        width,
                        val
                    ));
                }
                Event::Device { code } => s.push_str(&format!("E{code}")),
                Event::Internal { code } => s.push_str(&format!("E{code}")),
                Event::Fuel => s.push('F'),
                Event::Done { exit } => s.push_str(&format!("D{exit}")),
            }
        }
        if seen_deep {
            s.push_str(&format!("C{{…{deep_calls}}}"));
        }
        s
    }

    /// The event records (read-only; the debug projection's source).
    pub fn events(&self) -> &[Event] {
        &self.events
    }
}
