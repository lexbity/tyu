//! The wasm boundary (PLAN-WASM-1 P5, §Q7/§6.2): the bindgen surface the
//! browser page drives. This module compiles **exclusively** under
//! `cfg(target_arch = "wasm32")` (DEC-11) — host CI never builds or links
//! `wasm-bindgen`; the core API is exercised natively by everything in
//! P1–P4.
//!
//! Exported surface (bindgen methods on [`WasmRepl`], the §6.2 `repl_*`
//! functions re-homed onto the session object — bindgen's idiomatic shape):
//!
//! | §6.2 export      | method                                    |
//! |------------------|-------------------------------------------|
//! | `repl_new`       | `WasmRepl::new(sysroot_blob)` (constructor)|
//! | `repl_submit`    | `submit(src)`                              |
//! | `repl_step`      | `step(ops)`                                |
//! | `repl_stack`     | `stack()`                                  |
//! | `repl_ir_text`   | `ir_text()`                                |
//! | `repl_reset`     | `reset()`                                  |
//!
//! Imported surface (JS→wasm, under the `tyu` namespace the page installs):
//! `periph_read`, `periph_write`, `emit_output`, `now_ms`. `now_ms` is part
//! of the §6.2 contract but the engine NEVER calls it (UI timing only —
//! G55-a greps the engine modules for exactly this).
//!
//! Error discipline (§6.2): input-dependent failures are `SubmitResult`
//! statuses, never wasm traps. Only a malformed *sysroot blob* fails
//! `new` — as an `Err`, carrying E9153. A wasm trap is always an engine
//! bug: the panic hook fails the page loudly with the engine version (§10).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use codegen_core::compiled_desc::{
    decode_compiled_desc, validate_compiled_desc, CompiledDescriptor,
};

use wasm_bindgen::prelude::*;

// The lib is `#![no_std]`, but this module is the wasm32-only boundary: std
// exists on `wasm32-unknown-unknown` and the panic hook (§10) needs it.
extern crate std;

use crate::compile::SourceTable;
use crate::diag::code;
use crate::periph::{PeriphError, Peripheral};
use crate::session::{SessionOpts, Status};
use crate::sysroot::SysrootTable;
use crate::{ReplSession, SubmitResult};

// ---------------------------------------------------------------------------
// The imported surface (JS→wasm). The page installs `window.tyu` with these
// four functions; device implementations are first-party page code (§4.3).
// ---------------------------------------------------------------------------

#[wasm_bindgen]
extern "C" {
    /// Device read: MUST be a pure function of device state (§Q6). Called
    /// only for apertures the page has *bound* (see [`JsPeripheral`]).
    #[wasm_bindgen(js_namespace = tyu)]
    fn periph_read(ap: u16, off: u32, width: u8) -> u64;

    /// Device write: the same purity contract as `periph_read`.
    #[wasm_bindgen(js_namespace = tyu)]
    fn periph_write(ap: u16, off: u32, width: u8, val: u64);

    /// Diagnostic-channel bytes: streamed to the console pane by the page.
    #[wasm_bindgen(js_namespace = tyu)]
    fn emit_output(bytes: &[u8]);

    /// UI timing ONLY. Declared for the §6.2 contract; the engine never
    /// calls it (G55-a greps the engine modules for `now_ms` — this module
    /// is the single sanctioned mention).
    #[wasm_bindgen(js_namespace = tyu)]
    fn now_ms() -> f64;
}

/// Rust-side device dispatch: only apertures the page has *bound* reach the
/// JS device layer; an unbound aperture is `PeriphError::UnboundAperture`
/// (→ the engine's E9152 outcome) — exactly the [`crate::periph::Disabled`]
/// contract, so the E9152 surface never crosses into JS as an exception
/// (a JS exception here would be a wasm trap, violating §6.2's discipline).
struct JsPeripheral {
    bound: Vec<u16>,
}

impl Peripheral for JsPeripheral {
    fn read(&mut self, aperture: u16, off: u32, width: u8) -> Result<u64, PeriphError> {
        if self.bound.contains(&aperture) {
            Ok(periph_read(aperture, off, width))
        } else {
            Err(PeriphError::UnboundAperture)
        }
    }

    fn write(&mut self, aperture: u16, off: u32, width: u8, val: u64) -> Result<(), PeriphError> {
        if self.bound.contains(&aperture) {
            periph_write(aperture, off, width, val);
            Ok(())
        } else {
            Err(PeriphError::UnboundAperture)
        }
    }
}

// ---------------------------------------------------------------------------
// The panic hook (§10): a wasm trap is an engine bug — fail the page loudly
// with the engine version so a bug report identifies the artifact exactly.
// ---------------------------------------------------------------------------

#[wasm_bindgen]
extern "C" {
    /// `console.error` — the page-fail channel for the panic hook (§10).
    #[wasm_bindgen(js_namespace = console)]
    fn console_error(s: &str);
}

#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| {
        console_error(&format!(
            "tyu engine panic (E9158-class bug — do not ignore): {info} [engine v{}]",
            crate::ENGINE_VERSION
        ));
    }));
}

// ---------------------------------------------------------------------------
// The exported surface
// ---------------------------------------------------------------------------

/// The browser REPL session: a [`ReplSession`] whose imports resolve through
/// the shipped `tyu-sysroot/1` blob and whose device boundary dispatches to
/// the page's JS devices.
#[wasm_bindgen]
pub struct WasmRepl {
    session: ReplSession,
    search: SysrootSearch,
    descriptor: CompiledDescriptor,
    bound: Vec<u16>,
}

#[wasm_bindgen]
impl WasmRepl {
    /// `repl_new` (§6.2): parse the sysroot blob; any structural or hash
    /// violation is E9153 as an `Err` (input-dependent, never a trap).
    #[wasm_bindgen(constructor)]
    pub fn new(sysroot_blob: &[u8]) -> Result<WasmRepl, String> {
        let sysroot = crate::sysroot::decode_blob(sysroot_blob).map_err(|d| {
            format!(
                "E{} @{}..{}: {}",
                d.code,
                d.span.start,
                d.span.end,
                String::from_utf8_lossy(&d.message)
            )
        })?;
        let descriptor = decode_compiled_desc(include_bytes!(
            "../../../platforms/wasm32-unknown-unknown/platform.desc"
        ))
        .and_then(|desc| validate_compiled_desc(&desc).map(|()| desc))
        .map_err(|e| format!("E9153: invalid browser platform descriptor: {e:?}"))?;
        let mut session = ReplSession::new(SessionOpts::default());
        session.set_descriptor(Some(Box::new(descriptor)));
        Ok(WasmRepl {
            session,
            search: SysrootSearch { sysroot },
            descriptor,
            bound: Vec::new(),
        })
    }

    /// `repl_submit` (§6.2): the DEC-3..DEC-6 pipeline, synchronous.
    pub fn submit(&mut self, src: &str) -> JsSubmitResult {
        let tables: [&dyn SourceTable; 1] = [&self.search];
        let result = self.session.transactional_submit(src.as_bytes(), &tables);
        self.stream_output(&result);
        JsSubmitResult::of(&self.session, result)
    }

    /// Compile a fresh Run submission to a nested Wasm module. Execution is
    /// performed by the page's Wasm runtime so its imports share the same
    /// output and peripheral devices as interpreter mode.
    pub fn compile_backend(&self, src: &str) -> Result<Vec<u8>, String> {
        let tables: [&dyn SourceTable; 1] = [&self.search];
        crate::session::compile_backend_run(src.as_bytes(), &tables, &self.descriptor).map_err(
            |d| {
                format!(
                    "E{} @{}..{}: {}",
                    d.code,
                    d.span.start,
                    d.span.end,
                    String::from_utf8_lossy(&d.message)
                )
            },
        )
    }

    /// `repl_step` (§6.2): resume the fuel-suspended phrase with `ops` more
    /// ops of budget (§Q8 stepping). `ops == 0` would mean "unlimited" —
    /// rejected with E9151 on wasm (§Q6: the browser is never unlimited).
    pub fn step(&mut self, ops: u64) -> Result<JsSubmitResult, String> {
        if ops == 0 {
            return Err(e9151());
        }
        let result = self.session.step(ops);
        self.stream_output(&result);
        Ok(JsSubmitResult::of(&self.session, result))
    }

    /// Set the per-phrase fuel budget. `0` (= unlimited on host/CI) is
    /// rejected with E9151 on wasm (§Q6).
    pub fn set_fuel(&mut self, ops: u64) -> Result<(), String> {
        if ops == 0 {
            return Err(e9151());
        }
        self.session.set_fuel(ops);
        Ok(())
    }

    /// Bind an aperture to the page's JS device layer (P5's device
    /// registry; the page implements the device behavior). Unbound
    /// apertures stay E9152 inside the engine.
    pub fn bind_device(&mut self, aperture: u16) {
        if !self.bound.contains(&aperture) {
            self.bound.push(aperture);
        }
        self.session.set_peripheral(Box::new(JsPeripheral {
            bound: self.bound.clone(),
        }));
    }

    /// `repl_stack` (§6.2): the persistent stack, bottom-first.
    pub fn stack(&self) -> Vec<JsStackCell> {
        self.session
            .stack_cells()
            .into_iter()
            .map(|c| JsStackCell {
                ty_name: String::from_utf8_lossy(&c.ty_name).into_owned(),
                value_hex: String::from_utf8_lossy(&c.value_hex).into_owned(),
            })
            .collect()
    }

    /// `repl_ir_text` (§6.2): the last-good `--emit=ir` render (DEC-13).
    pub fn ir_text(&self) -> Option<String> {
        self.session
            .last_ir_text()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// The full accumulated output (the page appends the streamed
    /// `emit_output` bytes; this is the authoritative buffer).
    pub fn output_total(&self) -> Vec<u8> {
        self.session.out_total().to_vec()
    }

    /// `repl_reset` (§6.2): back to Fresh (§4.2).
    pub fn reset(&mut self) {
        self.session.reset();
    }

    /// The engine surface version (§10): the page pairs it with the artifact
    /// hash to identify a build exactly.
    pub fn engine_version(&self) -> u32 {
        crate::ENGINE_VERSION
    }

    // -----------------------------------------------------------------------
    // The W1 read-only snapshot surface (PLAN-WASM-2 §6.1, FR-27): thin
    // bindgen wrappers over `debug` — no logic here (R-E). Every export is
    // infallible for well-formed sessions; unknown names are structured
    // not-found values, never wasm traps (the §6.2 error discipline).
    // -----------------------------------------------------------------------

    /// `frames_snapshot`: the live frames of a fuel-suspended phrase
    /// (innermost-first, gdb order). Empty after a completed or rolled-back
    /// submission — the post-mortem backtrace is built from `trace_events`
    /// (§Q8; the UI selects the source by the last status).
    pub fn frames_snapshot(&self) -> Vec<JsFrameCell> {
        match (self.session.compiled(), self.session.suspended_state()) {
            (Some(c), Some(st)) => crate::debug::frames(c, st)
                .into_iter()
                .map(|f| {
                    let span = doc_span(&self.session, f.call_span);
                    JsFrameCell {
                        index: f.index,
                        word: String::from_utf8_lossy(&f.word).into_owned(),
                        sig: String::from_utf8_lossy(&f.sig).into_owned(),
                        call_span_start: span.start as u32,
                        call_span_end: span.end as u32,
                        locals: f
                            .locals
                            .iter()
                            .map(|c| JsStackCell {
                                ty_name: String::from_utf8_lossy(&c.ty_name).into_owned(),
                                value_hex: String::from_utf8_lossy(&c.value_hex).into_owned(),
                            })
                            .collect(),
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `trace_events`: the normalized trace of the LAST submission (DEC-20).
    /// Spans are doc-relative (I-9).
    pub fn trace_events(&self) -> Vec<JsTraceEvent> {
        let repl = &self.session;
        match self.session.last_trace() {
            Some(t) => crate::debug::trace_events(t)
                .into_iter()
                .map(|e| {
                    let (ap, off, width, val, is_write) = match e {
                        crate::debug::TraceEventView::Periph {
                            ap,
                            off,
                            width,
                            val,
                            is_write,
                        } => (ap, off, width, val, is_write),
                        _ => (0u16, 0u32, 0u8, 0u64, false),
                    };
                    let (kind, code, start, len, word, depth, detail, exit) = match e {
                        crate::debug::TraceEventView::Output { start, len } => {
                            ("output", 0u32, start, len, String::new(), 0u16, None, 0i64)
                        }
                        crate::debug::TraceEventView::Periph { .. } => {
                            // Flattened periph events carry their payload
                            // through `detail`-free scalar fields below.
                            ("periph", 0, 0, 0, String::new(), 0, None, 0)
                        }
                        crate::debug::TraceEventView::Call { word, depth } => (
                            "call",
                            0,
                            0,
                            0,
                            String::from_utf8_lossy(&word).into_owned(),
                            depth,
                            None,
                            0,
                        ),
                        crate::debug::TraceEventView::Trap { code, span, detail } => {
                            let span = doc_span(repl, span);
                            (
                                "trap",
                                code,
                                span.start as u32,
                                span.end.saturating_sub(span.start) as u32,
                                String::new(),
                                0,
                                detail.map(|d| JsTrapDetail {
                                    kind: String::from_utf8_lossy(d.kind.name()).into_owned(),
                                    word: String::from_utf8_lossy(&d.word).into_owned(),
                                    declared: String::from_utf8_lossy(&d.declared).into_owned(),
                                    actual: d
                                        .actual
                                        .iter()
                                        .map(|c| JsStackCell {
                                            ty_name: String::from_utf8_lossy(&c.ty_name)
                                                .into_owned(),
                                            value_hex: String::from_utf8_lossy(&c.value_hex)
                                                .into_owned(),
                                        })
                                        .collect(),
                                }),
                                0,
                            )
                        }
                        crate::debug::TraceEventView::Device { code } => {
                            ("device", code, 0, 0, String::new(), 0, None, 0)
                        }
                        crate::debug::TraceEventView::Internal { code } => {
                            ("internal", code, 0, 0, String::new(), 0, None, 0)
                        }
                        crate::debug::TraceEventView::Fuel => {
                            ("fuel", 0, 0, 0, String::new(), 0, None, 0)
                        }
                        crate::debug::TraceEventView::Done { exit } => {
                            ("done", 0, 0, 0, String::new(), 0, None, exit)
                        }
                    };
                    JsTraceEvent {
                        kind: kind.to_string(),
                        code,
                        span_start: start,
                        span_len: len,
                        word,
                        depth,
                        ap,
                        off,
                        width,
                        val,
                        is_write,
                        exit,
                        detail,
                    }
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// `words`: the session dictionary (completions, `?word`, the status
    /// count). The synthetic `repl-phrase` wrapper is machinery, not an
    /// entry.
    pub fn words(&self) -> Vec<JsWordInfo> {
        let sysroot: Option<&dyn crate::debug::SysrootFiles> = Some(&self.search.sysroot);
        match self.session.compiled() {
            Some(c) => crate::debug::words(c, self.session.source(), sysroot)
                .into_iter()
                .map(|w| {
                    // def_spans are SESSION-BUFFER offsets (the words() scan
                    // over the accumulated source — the I-9 anchor); raw,
                    // like the dump (I-11).
                    let (origin, def_start, def_len, path) = match &w.origin {
                        crate::debug::WordOrigin::Session { def_span } => (
                            "session",
                            def_span.start as u32,
                            def_span.end.saturating_sub(def_span.start) as u32,
                            String::new(),
                        ),
                        crate::debug::WordOrigin::Sysroot { path } => {
                            ("sysroot", 0, 0, String::from_utf8_lossy(path).into_owned())
                        }
                        crate::debug::WordOrigin::Builtin => ("builtin", 0, 0, String::new()),
                    };
                    JsWordInfo {
                        name: String::from_utf8_lossy(&w.name).into_owned(),
                        sig: String::from_utf8_lossy(&w.sig).into_owned(),
                        effects: String::from_utf8_lossy(&w.effects).into_owned(),
                        origin: origin.to_string(),
                        def_span_start: def_start,
                        def_span_len: def_len,
                        path,
                    }
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// `word_source`: view-source resolution (DEC-29). Sysroot from the
    /// blob's file map; session words resolve to the buffer (the UI shows
    /// the corresponding range); unknown names are `{origin:"not-found"}`.
    pub fn word_source(&self, name: &str) -> JsWordSource {
        let sysroot: Option<&dyn crate::debug::SysrootFiles> = Some(&self.search.sysroot);
        match crate::debug::word_source(
            self.session.compiled(),
            self.session.source(),
            sysroot,
            name.as_bytes(),
        ) {
            // def_span is a SESSION-BUFFER offset (the words() scan space).
            crate::debug::WordSource::Session { def_span } => JsWordSource {
                origin: "session".to_string(),
                path: String::new(),
                source: String::new(),
                def_span_start: def_span.start as u32,
                def_span_len: def_span.end.saturating_sub(def_span.start) as u32,
            },
            crate::debug::WordSource::Sysroot { path, source } => JsWordSource {
                origin: "sysroot".to_string(),
                path: String::from_utf8_lossy(&path).into_owned(),
                source: String::from_utf8_lossy(&source).into_owned(),
                def_span_start: 0,
                def_span_len: 0,
            },
            crate::debug::WordSource::NotFound => JsWordSource {
                origin: "not-found".to_string(),
                path: String::new(),
                source: String::new(),
                def_span_start: 0,
                def_span_len: 0,
            },
        }
    }

    /// `regions`: the region table (the heap inspector's rows, handle order).
    pub fn regions(&self) -> Vec<JsRegionInfo> {
        crate::debug::regions(self.session.heap())
            .into_iter()
            .map(|r| JsRegionInfo {
                id: r.id,
                off: r.off,
                len: r.len,
                cursor: r.cursor,
                kind: r.kind,
                alive: r.alive,
            })
            .collect()
    }

    /// `session_source`: the accumulated session buffer (the I-9 anchor the
    /// dictionary's definition spans index). The UI pairs it with
    /// `session word def_span` for view-source jumps.
    pub fn session_source(&self) -> Vec<u8> {
        self.session.source().to_vec()
    }

    /// `fuel_default`: the session's current default per-phrase budget —
    /// replaces the JS-hardcoded fuel constant (D-W9).
    pub fn fuel_default(&self) -> u64 {
        self.session.fuel().remaining()
    }

    /// `last_status`: the status bar's summary — a re-render without
    /// re-submitting (I-8: every field is ABI data).
    pub fn last_status(&self) -> JsStatusSummary {
        let st = crate::debug::status(
            self.session.stack_cells().len(),
            self.session.compiled(),
            self.session.fuel(),
        );
        let (status, exit_code, diag_code) = match self.session.last_status() {
            None => ("fresh".to_string(), 0i64, 0u32),
            Some(s) => {
                // One renderer (I-11): the outcome word is `Status::word`,
                // the same text the transcript and the debug dump print.
                let code = match s {
                    Status::CompileError { diag } | Status::Internal { diag } => diag.code,
                    Status::FuelExhausted { diag } => diag.code,
                    Status::DeviceError { code: c, .. } => *c,
                    Status::Trapped { code: c, .. } => ir::trap_code_u32(*c),
                    _ => 0,
                };
                (s.word(), 0i64, code)
            }
        };
        JsStatusSummary {
            status,
            exit_code,
            diag_code,
            stack_depth: st.stack_depth as u32,
            word_count: st.word_count as u32,
            fuel_ops: st.fuel_ops,
        }
    }

    /// Stream a result's new output bytes to the page (§6.2 `emit_output` —
    /// "streamed to the console pane AND retained in `out_total`"; the
    /// retention is the session's, the stream is this call).
    fn stream_output(&self, result: &SubmitResult) {
        let start = result.output_new.start as usize;
        let end = result.output_new.end as usize;
        let total = self.session.out_total();
        if start < end && end <= total.len() {
            emit_output(&total[start..end]);
        }
    }
}

/// The §Q12 search discipline over the single blob, mirroring the native
/// two-table layout (generic sysroot root first, then the hosted triple's
/// directory): a path is looked up as-is, then under the hosted triple
/// prefix. Today's sysroot has no root/triple filename collisions, so the
/// first-hit order is identical to the native two-table search.
struct SysrootSearch {
    sysroot: SysrootTable,
}

/// The hosted triple directory whose `.def`s resolve un-prefixed (DEC-8's
/// default target).
const HOSTED_TRIPLE: &str = "x86_64-unknown-linux-gnu/";

impl SourceTable for SysrootSearch {
    fn read(&self, path: &[u8]) -> Option<&[u8]> {
        if let Some(b) = self.sysroot.read(path) {
            return Some(b);
        }
        let mut prefixed = HOSTED_TRIPLE.as_bytes().to_vec();
        prefixed.extend_from_slice(path);
        self.sysroot.read(&prefixed)
    }
}

/// I-9: spans crossing the ABI are DOC-relative (the submitted buffer).
/// A span that lives in session machinery (the module prefix, the synthetic
/// phrase word's own tokens) has no doc home and collapses to UNKNOWN —
/// the UI never sees an anchor offset it cannot resolve.
fn doc_span(repl: &ReplSession, span: frontend::span::Span) -> frontend::span::Span {
    repl.doc_span(span).unwrap_or(frontend::span::Span::UNKNOWN)
}

fn e9151() -> String {
    format!(
        "E{}: unlimited fuel is rejected on wasm (§Q6)",
        code::E9151_UNLIMITED_FUEL
    )
}

/// The bindgen-compatible submit result (§6.2): a flattened, JS-friendly
/// projection of [`SubmitResult`]. `status` is one of `"ok"`,
/// `"compile-error"`, `"trapped"`, `"fuel"`, `"exited"`, `"device-error"`,
/// `"internal"`; `diag_code`/`diag_span_*`/`diag_message` carry the §7.5
/// envelope for every non-ok, non-exited status (compile diagnostics =
/// native codes verbatim; traps = frozen numerics; 9150+ = REPL layer).
#[wasm_bindgen]
pub struct JsSubmitResult {
    status: String,
    exit_code: i64,
    diag_code: u32,
    diag_span_start: u32,
    diag_span_end: u32,
    diag_message: String,
    detail: Option<JsTrapDetail>,
    output_start: u32,
    output_end: u32,
    ir_text: Option<String>,
}

#[wasm_bindgen]
impl JsSubmitResult {
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> String {
        self.status.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn exit_code(&self) -> i64 {
        self.exit_code
    }

    #[wasm_bindgen(getter)]
    pub fn diag_code(&self) -> u32 {
        self.diag_code
    }

    #[wasm_bindgen(getter)]
    pub fn diag_span_start(&self) -> u32 {
        self.diag_span_start
    }

    #[wasm_bindgen(getter)]
    pub fn diag_span_end(&self) -> u32 {
        self.diag_span_end
    }

    #[wasm_bindgen(getter)]
    pub fn diag_message(&self) -> String {
        self.diag_message.clone()
    }

    /// The §Q6 contract detail (FR-29): present ONLY on contract failures
    /// (trap 20); absent (`null`) on every other status.
    #[wasm_bindgen(getter)]
    pub fn detail(&self) -> Option<JsTrapDetail> {
        self.detail.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn output_start(&self) -> u32 {
        self.output_start
    }

    #[wasm_bindgen(getter)]
    pub fn output_end(&self) -> u32 {
        self.output_end
    }

    #[wasm_bindgen(getter)]
    pub fn ir_text(&self) -> Option<String> {
        self.ir_text.clone()
    }
}

impl JsSubmitResult {
    fn of(repl: &ReplSession, result: SubmitResult) -> JsSubmitResult {
        // The §7.5 envelope for every non-ok status: compile diagnostics are
        // native codes verbatim; traps are frozen numerics (I-3); 9150+ are
        // the REPL-layer registry.
        let diag: Option<crate::Diag> = match &result.status {
            Status::CompileError { diag } | Status::Internal { diag } => Some(diag.clone()),
            Status::FuelExhausted { diag } => Some(diag.clone()),
            Status::Trapped { code, span, .. } => Some(crate::Diag::from_bytes(
                ir::trap_code_u32(*code),
                *span,
                b"runtime trap (frozen numeric, I-3)",
            )),
            Status::DeviceError { code, span, .. } => Some(crate::Diag::from_bytes(
                *code,
                *span,
                &b"device-level failure (E9152 family)"[..],
            )),
            Status::Ok | Status::Exited { .. } => None,
        };
        let dstart = match &result.status {
            Status::CompileError { diag } | Status::Internal { diag } => doc_span(repl, diag.span),
            Status::FuelExhausted { diag } => doc_span(repl, diag.span),
            Status::Trapped { span, .. } => doc_span(repl, *span),
            Status::DeviceError { span, .. } => doc_span(repl, *span),
            _ => frontend::span::Span::UNKNOWN,
        };
        let detail = match &result.status {
            Status::Trapped {
                detail: Some(d), ..
            } => Some(JsTrapDetail {
                kind: String::from_utf8_lossy(d.kind.name()).into_owned(),
                word: String::from_utf8_lossy(&d.word).into_owned(),
                declared: String::from_utf8_lossy(&d.declared).into_owned(),
                actual: d
                    .actual
                    .iter()
                    .map(|c| JsStackCell {
                        ty_name: String::from_utf8_lossy(&c.ty_name).into_owned(),
                        value_hex: String::from_utf8_lossy(&c.value_hex).into_owned(),
                    })
                    .collect(),
            }),
            _ => None,
        };
        let status = match &result.status {
            Status::Ok => "ok",
            Status::Exited { .. } => "exited",
            Status::CompileError { .. } => "compile-error",
            Status::Trapped { .. } => "trapped",
            Status::FuelExhausted { .. } => "fuel",
            Status::DeviceError { .. } => "device-error",
            Status::Internal { .. } => "internal",
        };
        let exit_code = match &result.status {
            Status::Exited { code } => *code,
            _ => 0,
        };
        JsSubmitResult {
            status: status.to_string(),
            exit_code,
            diag_code: diag.as_ref().map(|d| d.code).unwrap_or(0),
            diag_span_start: dstart.start as u32,
            diag_span_end: dstart.end as u32,
            diag_message: diag
                .as_ref()
                .map(|d| String::from_utf8_lossy(&d.message).into_owned())
                .unwrap_or_default(),
            detail,
            output_start: result.output_new.start,
            output_end: result.output_new.end,
            ir_text: result
                .ir_text
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned()),
        }
    }
}

/// One stack cell for the UI (§6.2 `StackCell`): `{ty_name, value_hex}`,
/// bottom-first.
#[wasm_bindgen]
#[derive(Clone)]
pub struct JsStackCell {
    ty_name: String,
    value_hex: String,
}

#[wasm_bindgen]
impl JsStackCell {
    #[wasm_bindgen(getter)]
    pub fn ty_name(&self) -> String {
        self.ty_name.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn value_hex(&self) -> String {
        self.value_hex.clone()
    }
}

// ---------------------------------------------------------------------------
// The snapshot payload shapes (§6.1): flattened, JS-friendly projections of
// the `debug` structs. Read-only data; no callbacks, no traps.
// ---------------------------------------------------------------------------

#[wasm_bindgen]
pub struct JsFrameCell {
    pub(crate) index: u32,
    pub(crate) word: String,
    pub(crate) sig: String,
    pub(crate) call_span_start: u32,
    pub(crate) call_span_end: u32,
    pub(crate) locals: Vec<JsStackCell>,
}

#[wasm_bindgen]
impl JsFrameCell {
    #[wasm_bindgen(getter)]
    pub fn index(&self) -> u32 {
        self.index
    }
    #[wasm_bindgen(getter)]
    pub fn word(&self) -> String {
        self.word.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn sig(&self) -> String {
        self.sig.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn call_span_start(&self) -> u32 {
        self.call_span_start
    }
    #[wasm_bindgen(getter)]
    pub fn call_span_end(&self) -> u32 {
        self.call_span_end
    }
    #[wasm_bindgen(getter)]
    pub fn locals(&self) -> Vec<JsStackCell> {
        self.locals.clone()
    }
}

#[wasm_bindgen]
pub struct JsTraceEvent {
    pub(crate) kind: String,
    pub(crate) code: u32,
    pub(crate) span_start: u32,
    pub(crate) span_len: u32,
    pub(crate) word: String,
    pub(crate) depth: u16,
    pub(crate) ap: u16,
    pub(crate) off: u32,
    pub(crate) width: u8,
    pub(crate) val: u64,
    pub(crate) is_write: bool,
    pub(crate) exit: i64,
    pub(crate) detail: Option<JsTrapDetail>,
}

#[wasm_bindgen]
impl JsTraceEvent {
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn code(&self) -> u32 {
        self.code
    }
    #[wasm_bindgen(getter)]
    pub fn span_start(&self) -> u32 {
        self.span_start
    }
    #[wasm_bindgen(getter)]
    pub fn span_len(&self) -> u32 {
        self.span_len
    }
    #[wasm_bindgen(getter)]
    pub fn word(&self) -> String {
        self.word.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn depth(&self) -> u16 {
        self.depth
    }
    #[wasm_bindgen(getter)]
    pub fn ap(&self) -> u16 {
        self.ap
    }
    #[wasm_bindgen(getter)]
    pub fn off(&self) -> u32 {
        self.off
    }
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u8 {
        self.width
    }
    #[wasm_bindgen(getter)]
    pub fn val(&self) -> u64 {
        self.val
    }
    #[wasm_bindgen(getter)]
    pub fn is_write(&self) -> bool {
        self.is_write
    }
    #[wasm_bindgen(getter)]
    pub fn exit(&self) -> i64 {
        self.exit
    }
    #[wasm_bindgen(getter)]
    pub fn detail(&self) -> Option<JsTrapDetail> {
        self.detail.clone()
    }
}

#[wasm_bindgen]
#[derive(Clone)]
pub struct JsTrapDetail {
    pub(crate) kind: String,
    pub(crate) word: String,
    pub(crate) declared: String,
    pub(crate) actual: Vec<JsStackCell>,
}

#[wasm_bindgen]
impl JsTrapDetail {
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn word(&self) -> String {
        self.word.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn declared(&self) -> String {
        self.declared.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn actual(&self) -> Vec<JsStackCell> {
        self.actual.clone()
    }
}

#[wasm_bindgen]
pub struct JsWordInfo {
    pub(crate) name: String,
    pub(crate) sig: String,
    pub(crate) effects: String,
    pub(crate) origin: String,
    pub(crate) def_span_start: u32,
    pub(crate) def_span_len: u32,
    pub(crate) path: String,
}

#[wasm_bindgen]
impl JsWordInfo {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn sig(&self) -> String {
        self.sig.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn effects(&self) -> String {
        self.effects.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn origin(&self) -> String {
        self.origin.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn def_span_start(&self) -> u32 {
        self.def_span_start
    }
    #[wasm_bindgen(getter)]
    pub fn def_span_len(&self) -> u32 {
        self.def_span_len
    }
    #[wasm_bindgen(getter)]
    pub fn path(&self) -> String {
        self.path.clone()
    }
}

#[wasm_bindgen]
pub struct JsWordSource {
    pub(crate) origin: String,
    pub(crate) path: String,
    pub(crate) source: String,
    pub(crate) def_span_start: u32,
    pub(crate) def_span_len: u32,
}

#[wasm_bindgen]
impl JsWordSource {
    #[wasm_bindgen(getter)]
    pub fn origin(&self) -> String {
        self.origin.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn path(&self) -> String {
        self.path.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn source(&self) -> String {
        self.source.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn def_span_start(&self) -> u32 {
        self.def_span_start
    }
    #[wasm_bindgen(getter)]
    pub fn def_span_len(&self) -> u32 {
        self.def_span_len
    }
}

#[wasm_bindgen]
pub struct JsRegionInfo {
    pub(crate) id: u32,
    pub(crate) off: u32,
    pub(crate) len: u32,
    pub(crate) cursor: u32,
    pub(crate) kind: u8,
    pub(crate) alive: bool,
}

#[wasm_bindgen]
impl JsRegionInfo {
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> u32 {
        self.id
    }
    #[wasm_bindgen(getter)]
    pub fn off(&self) -> u32 {
        self.off
    }
    /// Named `size` (not `len`) — the clippy `len_without_is_empty` gate is
    /// part of G55 and a region row is not a collection.
    #[wasm_bindgen(getter)]
    pub fn size(&self) -> u32 {
        self.len
    }
    #[wasm_bindgen(getter)]
    pub fn cursor(&self) -> u32 {
        self.cursor
    }
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> u8 {
        self.kind
    }
    #[wasm_bindgen(getter)]
    pub fn alive(&self) -> bool {
        self.alive
    }
}

#[wasm_bindgen]
pub struct JsStatusSummary {
    pub(crate) status: String,
    pub(crate) exit_code: i64,
    pub(crate) diag_code: u32,
    pub(crate) stack_depth: u32,
    pub(crate) word_count: u32,
    pub(crate) fuel_ops: u64,
}

#[wasm_bindgen]
impl JsStatusSummary {
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> String {
        self.status.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn exit_code(&self) -> i64 {
        self.exit_code
    }
    #[wasm_bindgen(getter)]
    pub fn diag_code(&self) -> u32 {
        self.diag_code
    }
    #[wasm_bindgen(getter)]
    pub fn stack_depth(&self) -> u32 {
        self.stack_depth
    }
    #[wasm_bindgen(getter)]
    pub fn word_count(&self) -> u32 {
        self.word_count
    }
    #[wasm_bindgen(getter)]
    pub fn fuel_ops(&self) -> u64 {
        self.fuel_ops
    }
}
