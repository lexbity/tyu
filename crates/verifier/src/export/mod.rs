//! The port exporter (PLAN-VERIFY-3 P3.1, FR-12).
//!
//! Renders the *generated data layer* of every recognized port: op enum,
//! semantics table, target parameters, and the MemModel/DeviceModel/Services
//! interfaces. Hand-ported tables are disqualifying for port recognition
//! (developer-proof-pipeline.md §Q11 item 1) — the committed port files are
//! byte-drift-locked against this renderer
//! (`TYU_EXPORT_PORTS=1 cargo test -p verifier --test export_drift`).
//!
//! Layering (developer-proof-pipeline.md §5): `verification/ports/*`
//! consume generated artifacts; the generator edge (`verifier::export` →
//! committed port files) is **test-time only, drift-locked, never
//! build-time**.
//!
//! Renderers are pure functions of the row structs — no I/O, no nondeterminism
//! (FR-16: regeneration is byte-deterministic).

pub mod lean;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// One generated file of a port: a repo-relative path under
/// `verification/ports/` and its exact bytes.
pub struct GeneratedFile {
    /// Path relative to `verification/ports/<port>/` (e.g.
    /// `Tyu/IR/Op.lean`).
    pub path: &'static str,
    /// The full file content. Regeneration must be byte-identical
    /// (determinism test).
    pub content: String,
}

/// The port-renderer contract (P3.1): one renderer per recognized port.
pub trait PortRenderer {
    /// The port directory name under `verification/ports/` ("lean").
    fn port_name(&self) -> &'static str;
    /// Render every generated file of this port. Pure and deterministic.
    fn render(&self) -> Vec<GeneratedFile>;
}

/// Every recognized port renderer, in a deterministic order.
pub fn renderers() -> [&'static dyn PortRenderer; 1] {
    [&lean::LeanRenderer]
}

/// Render every port's generated files, keyed by port directory name.
pub fn render_all() -> Vec<(String, Vec<GeneratedFile>)> {
    renderers()
        .iter()
        .map(|r| (r.port_name().to_string(), r.render()))
        .collect()
}
