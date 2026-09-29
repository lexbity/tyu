//! `verify-report.json` model (static-verification.md §6.5, slice P3).
//!
//! Written by `tyu` after every image build (slice P3 v1 fields:
//! `modules`/`contexts`/`open`/`assumptions_trusted`; `assumed`, `retained`,
//! `provably_failing`, `stale_verdicts`, and `emitted_checks` arrive with the
//! slices that can honestly fill them — P4/P6). The report is deterministic
//! (FR-17): fixed key order, no maps anywhere, no timestamps.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// Schema identifier of `verify-report.json` artifacts.
///
/// v1 → v2 (PLAN-VERIFY-3 P6.2, §13): v2 adds the `proof` section — the
/// per-module statement accounting of the developer-proof pipeline (§Q5,
/// §7.3), defaulted until P7 lands the harvest + trust/method/surface
/// accounting. v1 consumers of the *open/assumed* surfaces are unaffected
/// (the new section is additive JSON members; the report has a single in-tree
/// writer, `tu` → `verifier::codec::encode_report`).
pub const REPORT_SCHEMA: &str = "tyu.verify-report/v2";

/// Schema identifier of the image-verdicts record (slice P7, Q5/FR-11): the
/// durable, validated evidence of the two-pass guard-elision decision —
/// what the report's `contexts.stack.guards` will say is exactly what this
/// record says (E6415 on a malformed record).
pub const IMAGE_VERDICTS_SCHEMA: &str = "tyu.image-verdicts/v1";

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

/// The v2 trust-class split for one module (§Q6, P7.3): *what kind of
/// evidence* discharged each closed obligation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct TrustAccounting {
    pub proof: u32,
    pub checked: u32,
    pub assumed: u32,
    pub open: u32,
}

/// The v2 closed-method accounting (§Q6): which discharge method closed each
/// obligation (the `rederive` count tells an auditor how much automation
/// trust the module rests on).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct MethodAccounting {
    pub certificate: u32,
    pub rederive: u32,
    pub descriptor: u32,
    pub stack_exact: u32,
    pub interval: u32,
}

/// The proof-surface accounting (§Q2, P7.3): certificates resting on the
/// source-level semantics (composition with T-S) vs the IR semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct SurfaceAccounting {
    pub source: u32,
    pub ir: u32,
}

/// One module's class accounting (P3 uses the five obligation kinds in fixed
/// order; zeroed kinds are emitted for a stable schema).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleAccounting {
    pub name: String,
    /// P12.1 (§7.3/§Q3): the module's `(target, model_semantics)` identity —
    /// the report's leg of the id-flow chain (pack → artifact → verdicts →
    /// manifest → package), copied from the artifact at report-composition
    /// time. `model` is `"unmodeled"` for a bundle without model semantics
    /// (§Q15); under `proven` an unmodeled module fails the build (E6510).
    pub target: String,
    pub model: String,
    /// P15.1 (§Q14/P7.3): the module's concurrency-service modeling
    /// declaration (`abstract-atomic` | `unmodeled`) — the report's leg of
    /// the artifact's statement relativism (bound by the FR-5 verdict
    /// identity).
    pub concurrency: String,
    pub classes: Vec<ClassAccounting>,
    /// P7.3: the v2 trust split across this module's obligations.
    pub trust: TrustAccounting,
    /// P7.3: the closed-method split.
    pub methods: MethodAccounting,
    /// P7.3: the certificate surface split (source/ir).
    pub surfaces: SurfaceAccounting,
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
    /// Slice P5: the open-reason from the interval engine (`"interval <top>
    /// vs target [0, 100]"`), when one was recorded. Omitted by the writer
    /// when `None` (schema-stable).
    pub reason: Option<String>,
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

/// One `provably_failing` record (`provably_failing` list in §6.5, slice P5):
/// the interval engine proved the site's value is always outside the target
/// range — the check is retained (never a discharge) and the report surfaces
/// why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvablyFailing {
    pub id: String,
    pub kind: String,
    pub module: String,
    pub word: String,
    pub site: String,
    pub line: u32,
    pub note: String,
}

/// One retained check record (`retained` list in §6.5, slice P6 FR-21):
/// a contract check the build kept for a *policy* reason even where a
/// discharge would have allowed elision — e.g. `retained (dynamic export)`
/// under the `module-loading` feature, where the loader's dynamic exports
/// are a runtime surface no build-time discharge may remove.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedObligation {
    pub id: String,
    pub reason: String,
}

/// The `verdict_sources` accounting (§6.5, slice P5): closed-verdict counts
/// by course — external verdicts file vs the in-tree dischargers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct VerdictSources {
    pub file: u32,
    pub in_tree: u32,
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

/// Per-module statement accounting of the proof pipeline (P6.2, §Q5/§7.3):
/// what the port's statement renderer generated for a module's obligations.
/// `rendered` = `def stmt_… : Prop` statements generated; `omitted` counts
/// the obligations the renderer refused (with a reason); `proven` (P7.1) is
/// how many rendered statements the harvest found kernel-checked theorems
/// for; `unproven` = `rendered − proven`. Before the P7 harvest, `proven`
/// was 0 by construction and the report was explicit about it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleStatements {
    pub module: String,
    pub rendered: u32,
    pub omitted: u32,
    /// P7.1: kernel-checked certificate counts from the harvest.
    pub proven: u32,
    pub unproven: u32,
    /// P10.2: of the harvested `proof` certificates, how many are
    /// candidate-authored (`authored: "candidate"` — fill-generated,
    /// unreviewed, kernel-gated). The report/package flag them so a reviewer
    /// knows what they are signing (§Q10).
    pub candidates: u32,
}

/// The report's proof section (P6.2; the §Q6 trust/method/surface accounting
/// lands with P7's verdicts v2 — this slice carries only the fields the
/// statement pipeline can honestly report before harvest exists).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofStatus {
    /// The verification tool active during this build: `"none"` (no
    /// `--verify-tool`), `"lean"` (the proof pipeline ran), or
    /// `"lean-skipped"` (the port build was skipped under
    /// `TYU_SKIP_PORT_BUILD=1`). Closed value set.
    pub tool: String,
    /// Harvest state: `"not-built"` in P6 (the port's harvest exe is P7's
    /// deliverable); `"ok"` from P7. Closed value set.
    pub harvest: String,
    /// Gen-digest check outcome (E6418): `"verified"` (every rendered
    /// statement's `statement_hash` matches the canonical encoder),
    /// `"skipped"` (no Gen surface to check — no `--verify-tool`, or the
    /// port build was skipped).
    pub gen_digest: String,
    /// SHA-256 (hex) over the vendored port library files as committed in the
    /// generated package — the §Q5 "digest-recorded in the build report"
    /// requirement: an auditor (or a later build) can re-derive it, and a
    /// tampered `Tyu/*.lean` in the package is a fail-closed E6418, not a
    /// silent "prove against the wrong semantics". Empty when no pipeline ran.
    pub vendor_digest: String,
    /// Per-module statement accounting, sorted by module name.
    pub statements: Vec<ModuleStatements>,
}

impl ProofStatus {
    /// The no-proof-pipeline default: `--verify-tool` absent.
    pub fn none() -> Self {
        Self {
            tool: "none".to_string(),
            harvest: "not-built".to_string(),
            gen_digest: "skipped".to_string(),
            vendor_digest: String::new(),
            statements: Vec::new(),
        }
    }

    /// The toolchain-skip status (`TYU_SKIP_PORT_BUILD=1`): the proof
    /// pipeline was requested but not attempted; every statement is unproven
    /// and the report says so.
    pub fn skipped() -> Self {
        Self {
            tool: "lean-skipped".to_string(),
            harvest: "not-built".to_string(),
            gen_digest: "skipped".to_string(),
            vendor_digest: String::new(),
            statements: Vec::new(),
        }
    }

    /// The P6 lean-pipeline status: the package was generated, the Gen
    /// digests verified (E6418), and the elaborating build ran — but no
    /// kernel-checked theorem was harvested yet (`harvest: "not-built"`), so
    /// every rendered statement is `unproven`.
    pub fn lean(gen_digest: &str, vendor_digest: &str, statements: Vec<ModuleStatements>) -> Self {
        Self {
            tool: "lean".to_string(),
            // P7: the harvest consumed the kernel environment (statements
            // without a kernel-checked theorem stay `unproven` — a state).
            harvest: "ok".to_string(),
            gen_digest: gen_digest.to_string(),
            vendor_digest: vendor_digest.to_string(),
            statements,
        }
    }
}

/// The shipped TCB boundary (§6.8, P7.3): what each trust-bearing component
/// of the pipeline is, and what status it holds (`assumed`, `reviewed+vectors`,
/// `drift-locked`, `theorem`, `structural`, `structural+modeled`,
/// `empirical`). Every `structural` entry is a named theorem the next cycle
/// must earn (formal-semantics-core).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TcbEntry {
    pub id: String,
    pub what: String,
    pub status: String,
}

/// The per-image assumption-closure status (§Q7 rule 2, P8.2): the report
/// face of the T-CL walker (`tyu::closure`). `well_closed` mirrors whether
/// every closed obligation's assumption graph terminated on closed /
/// `runtime-check` obligations; `unresolved` dependents were forced open
/// with witness `assumption-unresolved` and appear in the report's `open`
/// list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClosureStatus {
    /// The image's assumption graph is well-closed.
    pub well_closed: bool,
    /// Closed obligations whose assumption graphs were walked.
    pub checked: u32,
    /// Dependents forced open with witness `assumption-unresolved`.
    pub unresolved: u32,
}

impl Default for ClosureStatus {
    fn default() -> Self {
        Self {
            well_closed: true,
            checked: 0,
            unresolved: 0,
        }
    }
}

/// The complete report document (§6.5; P3 v1 fields plus the P4 honesty
/// fields `assumed`, `stale_verdicts`, `emitted_checks`, the P6 `proof`
/// section, and the P7.3 trust×method×surface accounting + TCB).
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
    /// Slice P5: interval-proven out-of-range sites (check retained — the
    /// `provably_failing` honesty diagnostic; never a discharge).
    pub provably_failing: Vec<ProvablyFailing>,
    /// Slice P6 (FR-21): checks retained for a policy reason (e.g. dynamic
    /// exports under `module-loading`), with the reason.
    pub retained: Vec<RetainedObligation>,
    /// Slope P5: closed-verdict course counts (file vs in-tree).
    pub verdict_sources: VerdictSources,
    /// Input verdicts that matched no obligation / disagreed on the hash
    /// (E6413-class staleness, fail-closed; FR-15).
    pub stale_verdicts: u32,
    /// The honesty block: emitted checks per class (FR-15/FR-16).
    pub emitted_checks: EmittedChecks,
    /// P6.2: the proof-pipeline section (per-module statement accounting;
    /// trust/method/surface fields land in P7). `"none"` when the build ran
    /// without `--verify-tool`.
    pub proof: ProofStatus,
    /// P8.2: the per-image assumption-closure status (§Q7 rule 2) — the
    /// T-CL walker's outcome over the merged verdict sets.
    pub closure: ClosureStatus,
    /// P7.3: the shipped TCB boundary (§6.8), in fixed order.
    pub tcb: Vec<TcbEntry>,
}

impl VerifyReport {
    /// The default TCB (§6.8): structural entries now, theorem statuses
    /// filled as phases land (T-C/T-S/T-CL proof → `theorem`).
    pub fn default_tcb() -> Vec<TcbEntry> {
        vec![
            TcbEntry {
                id: "T-F1(lean)".into(),
                what: "lean4 kernel + pinned toolchain + lean4checker".into(),
                status: "assumed".into(),
            },
            TcbEntry {
                id: "T-F2(lean)".into(),
                what: "authored port definitions (Step/Mem/Src/Abs)".into(),
                status: "reviewed+vectors".into(),
            },
            TcbEntry {
                id: "T-F3".into(),
                what: "Rust→port generators (export renderers)".into(),
                status: "drift-locked".into(),
            },
            TcbEntry {
                id: "T-S".into(),
                what: "source→IR transcription (pure fragment)".into(),
                status: "theorem".into(),
            },
            TcbEntry {
                id: "T-CL".into(),
                what: "assumption closure".into(),
                status: "theorem".into(),
            },
            TcbEntry {
                id: "T-C".into(),
                what: "stack algebra".into(),
                status: "theorem".into(),
            },
            TcbEntry {
                id: "STRUCT-borrow".into(),
                what: "borrow exclusivity/escape — checker alias analysis".into(),
                status: "structural".into(),
            },
            TcbEntry {
                id: "STRUCT-effects".into(),
                what: "context×effect matrix — ContextStack checker".into(),
                status: "structural".into(),
            },
            TcbEntry {
                id: "STRUCT-lock".into(),
                what: "lock lowering + ISR isolation + cross-context rule".into(),
                status: "structural".into(),
            },
            TcbEntry {
                id: "STRUCT-iso".into(),
                what: "iso linearity — checker".into(),
                status: "structural".into(),
            },
            TcbEntry {
                id: "STRUCT-sched".into(),
                what: "scheduler/runtime asm — abstract-atomic model".into(),
                status: "structural+modeled".into(),
            },
            TcbEntry {
                id: "T-F4'".into(),
                what: "IR→machine code (codegen) — conformance + QEMU".into(),
                status: "empirical".into(),
            },
        ]
    }

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
            provably_failing: Vec::new(),
            retained: Vec::new(),
            verdict_sources: VerdictSources::default(),
            stale_verdicts: 0,
            emitted_checks: EmittedChecks::default(),
            proof: ProofStatus::none(),
            closure: ClosureStatus::default(),
            tcb: VerifyReport::default_tcb(),
        }
    }
}
