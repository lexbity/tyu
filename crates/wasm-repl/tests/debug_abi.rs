//! The debug data substrate (PLAN-WASM-2 W1, AC-W1): the `debug` snapshot
//! projections the native CLI (`debug-dump`) and the wasm ABI both wrap
//! (R-E / I-11). Asserted on the core API directly — the ABI is a thin
//! wrapper, the CLI a thin JSON encoder; this file is the one native pin.

mod common;

use std::sync::Arc;

use wasm_repl::compile::SourceTable;
use wasm_repl::debug::{
    self, FrameCell, RegionInfo, SessionStatus, TraceEventView, WordOrigin, WordSource,
};
use wasm_repl::engine::{DetailKind, TrapDetail};
use wasm_repl::{ReplSession, SessionOpts, Status};
use wasm_repl::{StackCell, Trace};

type Table = Arc<common::MemTable>;

fn refs(s: &[Table]) -> Vec<&dyn SourceTable> {
    common::search_refs(s)
}

/// A session with a tiny default budget (fuel suspension drives the live
/// frames case deterministically).
fn session(fuel: u64) -> ReplSession {
    ReplSession::new(SessionOpts {
        fuel: wasm_repl::Fuel::new(fuel),
        ..SessionOpts::default()
    })
}

fn submit(sess: &mut ReplSession, src: &[u8]) -> Status {
    let s = common::sysroot_search();
    let r = refs(&s);
    sess.transactional_submit(src, &r).status
}

/// The test-local sysroot file map (the DEC-29 resolver's harness backing).
struct SysrootMap(Vec<(Vec<u8>, Vec<u8>)>);

impl debug::SysrootFiles for SysrootMap {
    fn files(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.0.clone()
    }
}

fn sysroot_map() -> SysrootMap {
    let s = common::sysroot_search();
    // The search is [root, triple-dir]; both tables' files, prefixed like the
    // compiler resolves them (the root table already carries canonical paths).
    let mut files: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let root = common::sysroot_dir();
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<(Vec<u8>, Vec<u8>)>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else if p.extension().map(|x| x == "def").unwrap_or(false) {
                    let rel = p
                        .strip_prefix(base)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/");
                    if let Ok(bytes) = std::fs::read(&p) {
                        out.push((rel.into_bytes(), bytes));
                    }
                }
            }
        }
    }
    walk(&root, &root, &mut files);
    files.sort();
    let _ = s;
    SysrootMap(files)
}

// ---------------------------------------------------------------------------
// (a) frames_snapshot: live frames of a suspended phrase (§Q8's live case)
// ---------------------------------------------------------------------------

#[test]
fn frames_of_a_suspended_phrase_are_innermost_first() {
    let mut sess = session(30);
    assert_eq!(
        submit(&mut sess, b": loopword ( i64 -- i64 ) 1 + loopword ;"),
        Status::Ok
    );
    let st = submit(&mut sess, b"5 loopword");
    assert!(
        matches!(st, Status::FuelExhausted { .. }),
        "expected suspension, got {st:?}"
    );
    let compiled = sess.compiled().expect("compiled");
    let state = sess.suspended_state().expect("suspended state has frames");
    let frames: Vec<FrameCell> = debug::frames(compiled, state);
    assert!(frames.len() >= 2, "phrase + recursive frames: {:?}", frames);
    // Innermost first (gdb order): frame 0 is the deepest `loopword`.
    assert_eq!(frames[0].index, 0);
    assert_eq!(frames[0].word, b"loopword");
    assert_eq!(frames[0].sig, b"( i64 -- i64 )");
    // Every frame above the entry is the same recursion, in reverse depth
    // order; the entry frame is the synthetic phrase word.
    let (entry, rest) = frames.split_last().expect("non-empty");
    assert_eq!(entry.word, b"repl-phrase");
    for (i, f) in rest.iter().enumerate() {
        assert_eq!(f.index as usize, i);
        assert_eq!(f.word, b"loopword");
    }
    // Call spans resolve into the compiled module text (the as-built span
    // anchor; the I-9 encoding is (start,len) either way): slicing it at a
    // recursive frame's call span yields the call site text.
    let src = &compiled.src;
    let span = frames[1].call_span;
    assert_ne!(span, ir::Span::UNKNOWN, "recursive call site known");
    assert_eq!(&src[span.start..span.end], b"loopword");
    // The entry frame was not called: its call span is UNKNOWN.
    let last = frames.last().unwrap();
    assert_eq!(last.call_span, ir::Span::UNKNOWN);
}

#[test]
fn frames_are_absent_without_a_suspension() {
    let mut sess = session(1_000_000);
    assert_eq!(submit(&mut sess, b": f ( -- ) ;"), Status::Ok);
    assert_eq!(submit(&mut sess, b"f"), Status::Ok);
    assert!(sess.suspended_state().is_none());
}

// ---------------------------------------------------------------------------
// (b) trace_events: the last submission's normalized trace, contract detail
// ---------------------------------------------------------------------------

#[test]
fn trace_events_of_a_contract_trap_carry_the_detail() {
    let mut sess = session(1_000_000);
    assert_eq!(
        submit(&mut sess, b": no-sevens ( i64 -- i64 bool ) dup 7 == not ;"),
        Status::Ok
    );
    assert_eq!(
        submit(
            &mut sess,
            b": add-safely ( i64 -- i64 ) needs [ no-sevens ] 1 + ;"
        ),
        Status::Ok
    );
    let st = submit(&mut sess, b"7 add-safely drop");
    let Status::Trapped { code, detail, .. } = st else {
        panic!("expected the contract trap, got {st:?}");
    };
    assert_eq!(ir::trap_code_u32(code), 20);
    let detail: TrapDetail = detail.expect("trap 20 carries the Q6 detail");
    assert_eq!(detail.kind, DetailKind::SigMismatch);
    assert_eq!(detail.kind.name(), b"sig-mismatch");
    assert_eq!(detail.word, b"add-safely");
    assert_eq!(detail.declared, b"( i64 -- i64 )");
    // The actual cells are engine-derived StackCells: the guard observed the
    // input and the false contract result.
    assert!(detail
        .actual
        .iter()
        .any(|c: &StackCell| c.ty_name == b"i64" && c.value_hex == b"0x0000000000000007"));

    // The trace_events projection mirrors it: Call chain + terminal Trap.
    let trace: &Trace = sess.last_trace().expect("last submission had a phrase");
    let events: Vec<TraceEventView> = debug::trace_events(trace);
    let calls: Vec<&Vec<u8>> = events
        .iter()
        .filter_map(|e| match e {
            TraceEventView::Call { word, .. } => Some(word),
            _ => None,
        })
        .collect();
    assert!(calls.contains(&&b"add-safely".to_vec()));
    assert!(calls.contains(&&b"no-sevens".to_vec()));
    let trap = events
        .iter()
        .find_map(|e| match e {
            TraceEventView::Trap { code, detail, .. } => Some((*code, detail.clone())),
            _ => None,
        })
        .expect("terminal trap event");
    assert_eq!(trap.0, 20);
    let tdetail = trap.1.expect("contract trap detail in the trace view");
    assert_eq!(tdetail.kind.name(), b"sig-mismatch");
    assert_eq!(tdetail.word, b"add-safely");
}

#[test]
fn non_contract_traps_have_no_detail() {
    let mut sess = session(1_000_000);
    assert_eq!(
        submit(&mut sess, b": reloop ( i64 -- i64 ) 1 + reloop ;"),
        Status::Ok
    );
    let st = submit(&mut sess, b"0 reloop");
    let Status::Trapped { code, detail, .. } = st else {
        panic!("expected the depth trap, got {st:?}");
    };
    assert_eq!(ir::trap_code_u32(code), 10);
    assert!(detail.is_none(), "trap 10 carries no Q6 detail");
    let trace = sess.last_trace().unwrap();
    let events = debug::trace_events(trace);
    let trap = events
        .iter()
        .find_map(|e| match e {
            TraceEventView::Trap { detail, .. } => Some(detail.clone()),
            _ => None,
        })
        .unwrap();
    assert!(trap.is_none());
    // The header records the FR-35 facts.
    assert_eq!(trace.header.v, 1);
    assert_eq!(trace.header.spec, "x86_64-unknown-linux-gnu");
    assert_eq!(trace.header.fuel, 1_000_000);
    assert_eq!(trace.header.source_hash.len(), 64);
}

// ---------------------------------------------------------------------------
// (c) words(): dictionary entries with buffer-resolvable definition spans
// ---------------------------------------------------------------------------

#[test]
fn words_carry_session_definition_spans() {
    let mut sess = session(1_000_000);
    assert_eq!(
        submit(&mut sess, b": double ( i64 -- i64 ) dup + ;"),
        Status::Ok
    );
    let words = debug::words(sess.compiled().unwrap(), sess.source(), None);
    let double = words
        .iter()
        .find(|w| w.name == b"double")
        .expect("session word in the dictionary");
    assert_eq!(double.sig, b"( i64 -- i64 )");
    match &double.origin {
        WordOrigin::Session { def_span } => {
            let src = sess.source();
            assert_eq!(&src[def_span.start..def_span.end], b"double");
        }
        other => panic!("expected a session origin, got {other:?}"),
    }
    // The synthetic phrase wrapper is machinery, not a dictionary entry.
    assert!(words.iter().all(|w| w.name != b"repl-phrase"));
}

// ---------------------------------------------------------------------------
// (d) word_source: session → buffer, sysroot → blob file, unknown → NotFound
// ---------------------------------------------------------------------------

#[test]
fn word_source_resolves_all_three_ways() {
    let mut sess = session(1_000_000);
    assert_eq!(
        submit(&mut sess, b": double ( i64 -- i64 ) dup + ;"),
        Status::Ok
    );
    let map = sysroot_map();
    assert!(!map.0.is_empty(), "the hosted sysroot has .def files");

    // Session word → its buffer span.
    match debug::word_source(sess.compiled(), sess.source(), Some(&map), b"double") {
        WordSource::Session { def_span } => {
            assert_eq!(&sess.source()[def_span.start..def_span.end], b"double");
        }
        other => panic!("expected session resolution, got {other:?}"),
    }
    // Sysroot word → the defining file (the blob's `.def` sources use the
    // same `: name` definition convention).
    let sysroot_words = ["_CONTRACT_FAIL", "_ASSERT_FAIL"];
    let hit = sysroot_words
        .iter()
        .filter_map(|n| {
            match debug::word_source(sess.compiled(), sess.source(), Some(&map), n.as_bytes()) {
                WordSource::Sysroot { path, source } => Some((path, source)),
                _ => None,
            }
        })
        .next()
        .expect("a sysroot word resolves to its defining file");
    assert!(hit.0.ends_with(b".def"));
    assert!(!hit.1.is_empty());
    // Unknown → structured not-found, never a guess.
    assert_eq!(
        debug::word_source(sess.compiled(), sess.source(), Some(&map), b"no-such-word"),
        WordSource::NotFound
    );
    // No sysroot at all: session words still resolve; everything else not.
    assert!(matches!(
        debug::word_source(sess.compiled(), sess.source(), None, b"double"),
        WordSource::Session { .. }
    ));
    assert_eq!(
        debug::word_source(sess.compiled(), sess.source(), None, b"_CONTRACT_FAIL"),
        WordSource::NotFound
    );
}

// ---------------------------------------------------------------------------
// (e) regions(): the region table after region ops
// ---------------------------------------------------------------------------

#[test]
fn regions_reflect_the_committed_heap() {
    let mut sess = session(1_000_000);
    assert_eq!(submit(&mut sess, b"import platform/mem;"), Status::Ok);
    assert_eq!(
        submit(&mut sess, b"64 as usize platform.mem.region-create"),
        Status::Ok
    );
    let regions: Vec<RegionInfo> = debug::regions(sess.heap());
    let live: Vec<&RegionInfo> = regions.iter().filter(|r| r.alive).collect();
    assert_eq!(live.len(), 1, "one live region: {regions:?}");
    assert_eq!(live[0].len, 64);
    assert_eq!(live[0].cursor, 0, "nothing allocated from it yet");
    assert_eq!(live[0].kind, 0, "the as-built table carries no kind tag");
    // A rolled-back creation leaves no live region (DEC-5 reclamation): the
    // phrase exhausts its fuel after the second create and rolls back.
    sess.set_fuel(2);
    let st = submit(&mut sess, b"32 as usize platform.mem.region-create");
    assert!(matches!(st, Status::FuelExhausted { .. }), "got {st:?}");
    let regions = debug::regions(sess.heap());
    assert_eq!(regions.iter().filter(|r| r.alive).count(), 1);
}

// ---------------------------------------------------------------------------
// (f) status + empty/unknown inputs: structured, never a panic
// ---------------------------------------------------------------------------

#[test]
fn status_and_empty_inputs_are_structured() {
    let mut sess = session(1_000_000);
    // Fresh session: empty everything.
    assert!(sess.compiled().is_none());
    assert!(sess.last_trace().is_none());
    assert!(sess.last_status().is_none());
    assert!(debug::regions(sess.heap()).iter().all(|r| !r.alive));
    let st: SessionStatus = debug::status(0, None, sess.fuel());
    assert_eq!(st.stack_depth, 0);
    assert_eq!(st.word_count, 0);
    assert_eq!(st.fuel_ops, 1_000_000);

    // After a submit: the status snapshot reflects the session.
    assert_eq!(submit(&mut sess, b": f ( -- ) ; 1 2 +"), Status::Ok);
    let st = debug::status(sess.stack_cells().len(), sess.compiled(), sess.fuel());
    assert_eq!(st.stack_depth, 1);
    // One module word plus the checker's operator builtins (dup/drop/+/… —
    // the W5 dictionary includes the full introspectable surface).
    assert_eq!(st.word_count, 1 + 16);
    assert!(sess.last_status().is_some());
}
