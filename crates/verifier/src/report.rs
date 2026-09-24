//! `verify-report.json` model (static-verification.md §6.5, slice P3).
//!
//! Written by `tyu` after every image build (slice P3 v1 fields:
//! `modules`/`contexts`/`open`/`assumptions_trusted`; `assumed`, `retained`,
//! `provably_failing`, `stale_verdicts`, and `emitted_checks` arrive with the
//! slices that can honestly fill them — P4/P6). The report is deterministic
//! (FR-17): fixed key order, no maps anywhere, no timestamps.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Schema identifier of `verify-report.json` artifacts.
pub const REPORT_SCHEMA: &str = "tyu.verify-report/v1";

/// Tool/product identity (`tool` in §6.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

/// Per-kind obligation accounting for one module (`classes` in §6.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassAccounting {
    pub kind: String,
    pub total: u32,
    pub discharged: u32,
    pub assumed: u32,
    pub open: u32,
}

impl ClassAccounting {
    pub fn zero(kind: &'static str) -> Self {
        Self {
            kind: kind.to_string(),
            total: 0,
            discharged: 0,
            assumed: 0,
            open: 0,
        }
    }
}

/// One module's class accounting (P3 uses the five obligation kinds in fixed
/// order; zeroed kinds are emitted for a stable schema).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleAccounting {
    pub name: String,
    pub classes: Vec<ClassAccounting>,
}

/// The `contexts.stack` section (P3): per-context budget verdicts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StackContextAccounting {
    pub main: MainContextAccounting,
    pub isr: IsrContextAccounting,
    /// `"retained"` in P3 — guard elision is a P7 decision.
    pub guards: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MainContextAccounting {
    /// Peak slots of the image's `main` word (`0xFFFF_FFFF` when `top`).
    pub high: u32,
    pub top: bool,
    /// `N_main` — the declared grant; 0 when the pack declares none.
    pub budget: u32,
    /// `"discharged"` | `"open"` | `"n/a"` (P4+ adds `"assumed"`).
    pub verdict: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsrContextAccounting {
    /// Max peak over the image's ISR handler words.
    pub max_high: u32,
    /// `N_isr` (default 32 when the pack omits it; FR-10).
    pub budget: u32,
    pub handlers: u32,
    /// `"discharged"` | `"n/a"` (no handlers).
    pub verdict: String,
}

/// One open obligation (`open` list in §6.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenObligation {
    pub id: String,
    pub kind: String,
    pub module: String,
    pub word: String,
    /// `"<word>.<occurrence>"` — site shorthand.
    pub site: String,
    pub line: u32,
}

/// One assumed obligation (`assumed` list in §6.5, slice P4): a human
/// decision, recorded with its justification, not a proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssumedObligation {
    pub id: String,
    pub kind: String,
    pub module: String,
    pub word: String,
    pub site: String,
    pub line: u32,
    pub justification: Option<String>,
}

/// The image-level `emitted_checks` honesty block (§6.5, slice P4): how many
/// runtime checks of each class are actually in the produced object. MUST
/// agree with the object code (FR-16 bijection test). Aggregated from each
/// module's verdict echo (`EmittedChecksData`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct EmittedChecks {
    pub subtype_range: u32,
    pub contract: u32,
    pub mmio_bounds: u32,
    /// `true` when x86_64 data-stack guards are present in the image (P7
    /// elides them; P3/P4 image builds keep them).
    pub data_stack_guards: bool,
}

/// One trusted descriptor fact used by a discharge (`assumptions_trusted`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedAssumption {
    pub kind: String,
    pub what: String,
}

/// The complete report document (§6.5; P3 v1 fields plus the P4 honesty
/// fields `assumed`, `stale_verdicts`, `emitted_checks`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyReport {
    pub schema: String,
    pub tool: ToolInfo,
    pub semantics: String,
    pub policy: String,
    pub modules: Vec<ModuleAccounting>,
    pub contexts: StackContextAccounting,
    pub open: Vec<OpenObligation>,
    pub assumed: Vec<AssumedObligation>,
    pub assumptions_trusted: Vec<TrustedAssumption>,
    /// Input verdicts that matched no obligation / disagreed on the hash
    /// (E6413-class staleness, fail-closed; FR-15).
    pub stale_verdicts: u32,
    /// The honesty block: emitted checks per class (FR-15/FR-16).
    pub emitted_checks: EmittedChecks,
}

impl VerifyReport {
    /// A fresh report skeleton with the fixed top-level fields; slices fill
    /// the per-build data.
    pub fn new(tool: ToolInfo, policy: &str) -> Self {
        Self {
            schema: REPORT_SCHEMA.to_string(),
            tool,
            semantics: crate::semantics::SEMANTICS_VERSION.to_string(),
            policy: policy.to_string(),
            modules: Vec::new(),
            contexts: StackContextAccounting {
                main: MainContextAccounting {
                    high: 0,
                    top: false,
                    budget: 0,
                    verdict: "n/a".to_string(),
                },
                isr: IsrContextAccounting {
                    max_high: 0,
                    budget: 0,
                    handlers: 0,
                    verdict: "n/a".to_string(),
                },
                guards: "retained".to_string(),
            },
            open: Vec::new(),
            assumed: Vec::new(),
            assumptions_trusted: Vec::new(),
            stale_verdicts: 0,
            emitted_checks: EmittedChecks::default(),
        }
    }
}