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
use crate::journal::Effect;

/// The trace schema version (fail-closed: consumers reject unknown versions).
pub const TRACE_SCHEMA: &str = "tyu.trace/1";

/// One derived trace event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// Diagnostic-channel bytes for the range `[start..start+len)` of the
    /// phrase output.
    Output { start: u32, len: u32 },
    /// A `Call` with the depth before the call (call pane).
    Call { word: String, depth: u16 },
    /// A trap with its frozen code + the faulting op's span.
    Trap { code: TrapCode, span: Span },
    /// Fuel exhausted (E9150 surface).
    Fuel,
    /// The run's exit (natural completion = 0; `testio.exit` passes its code).
    Done { exit: i64 },
}

/// A phrase trace, derived from the journal and the outcome.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    events: Vec<Event>,
}

impl Trace {
    pub fn new() -> Trace {
        Trace { events: Vec::new() }
    }

    /// Project a phrase's observable trace: journal `Output`/`Call` events
    /// plus the run outcome's trap/fuel/done tail.
    pub fn from_phrase(journal_effects: &[Effect], outcome: &Outcome) -> Trace {
        let mut t = Trace::new();
        for e in journal_effects {
            match e {
                Effect::Output { start, len } => t.events.push(Event::Output {
                    start: *start,
                    len: *len,
                }),
                Effect::Call { word, depth_before } => t.events.push(Event::Call {
                    word: String::from_utf8_lossy(word).into_owned(),
                    depth: *depth_before,
                }),
                _ => {}
            }
        }
        match outcome {
            Outcome::Ok { .. } => t.events.push(Event::Done { exit: 0 }),
            Outcome::Exited { code, .. } => t.events.push(Event::Done { exit: *code }),
            Outcome::Trap { code, span, .. } => t.events.push(Event::Trap {
                code: *code,
                span: *span,
            }),
            Outcome::Fuel { .. } => t.events.push(Event::Fuel),
        }
        t
    }

    /// The normalized comparison text (golden-comparable, §7.4): event
    /// structure only — no timings, no fuel-internal counters. Call events
    /// above depth 32 are elided with a single `…` marker (the §7.4
    /// bound — a >32-deep tail renders as `C{…}`, not per-frame noise).
    pub fn normalize(&self) -> String {
        let mut s = String::new();
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
                Event::Trap { code, .. } => s.push_str(&format!("T{:?}", code)),
                Event::Fuel => s.push('F'),
                Event::Done { exit } => s.push_str(&format!("D{exit}")),
            }
        }
        if seen_deep {
            s.push_str(&format!("C{{…{deep_calls}}}"));
        }
        s
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }
}
