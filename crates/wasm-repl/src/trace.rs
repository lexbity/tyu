//! The `tyu.trace/1` event trace (PLAN-WASM-1 §7.4): event-oriented, so a
//! 100 M-op phrase never becomes 100 M records. Goldens compare *normalized*
//! traces — output bytes, peripheral event sequences, trap codes/spans, exit
//! codes — never timings, never fuel-internal counters.
//!
//! P2's observable surface (G53) is stdout bytes + exit/trap codes; the call
//! pane and periph events are emitted here so later slices (P3/P4) reuse the
//! same writer without a format change.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use ir::TrapCode;

/// The trace schema version (fail-closed: consumers reject unknown versions).
pub const TRACE_SCHEMA: &str = "tyu.trace/1";

/// One trace event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// Diagnostic-channel bytes for the range `[start..start+len)`.
    Output { start: u32, len: u32 },
    /// A `Call` into a word (call pane; bounded depth below).
    Call { word: String, depth: u16 },
    /// A trap with its frozen code.
    Trap { code: TrapCode },
    /// The run's exit code (natural completion = 0).
    Done { exit: i64 },
}

/// A trace being built during a run.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    events: Vec<Event>,
    /// Call events are elided above this depth with a `...` marker (§7.4).
    pub output_len: u32,
}

impl Trace {
    pub fn new() -> Trace {
        Trace {
            events: Vec::new(),
            output_len: 0,
        }
    }

    pub fn record(&mut self, e: Event) {
        self.events.push(e);
    }

    /// The normalized comparison text (golden-comparable).
    pub fn normalize(&self) -> String {
        // Output/call events are the observable; internals are excluded.
        let mut s = alloc::string::String::new();
        for e in &self.events {
            match e {
                Event::Output { .. } => s.push('O'),
                Event::Call { word, depth } => {
                    s.push_str(&format!("C{{{word}:{depth}}}"));
                }
                Event::Trap { code } => {
                    s.push_str(&format!("T{:?}", code));
                }
                Event::Done { exit } => {
                    s.push_str(&format!("D{exit}"));
                }
            }
        }
        s
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }
}
