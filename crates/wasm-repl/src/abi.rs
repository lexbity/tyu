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
        Ok(WasmRepl {
            session: ReplSession::new(SessionOpts::default()),
            search: SysrootSearch { sysroot },
            bound: Vec::new(),
        })
    }

    /// `repl_submit` (§6.2): the DEC-3..DEC-6 pipeline, synchronous.
    pub fn submit(&mut self, src: &str) -> JsSubmitResult {
        let tables: [&dyn SourceTable; 1] = [&self.search];
        let result = self.session.transactional_submit(src.as_bytes(), &tables);
        self.stream_output(&result);
        JsSubmitResult::of(result)
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
        Ok(JsSubmitResult::of(result))
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
    fn of(result: SubmitResult) -> JsSubmitResult {
        // The §7.5 envelope for every non-ok status: compile diagnostics are
        // native codes verbatim; traps are frozen numerics (I-3); 9150+ are
        // the REPL-layer registry.
        let diag: Option<crate::Diag> = match &result.status {
            Status::CompileError { diag } | Status::Internal { diag } => Some(diag.clone()),
            Status::FuelExhausted { diag } => Some(diag.clone()),
            Status::Trapped { code, span } => Some(crate::Diag::from_bytes(
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
            diag_span_start: diag.as_ref().map(|d| d.span.start as u32).unwrap_or(0),
            diag_span_end: diag.as_ref().map(|d| d.span.end as u32).unwrap_or(0),
            diag_message: diag
                .as_ref()
                .map(|d| String::from_utf8_lossy(&d.message).into_owned())
                .unwrap_or_default(),
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
