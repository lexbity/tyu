//! Verdicts file codec — `tyu.verdicts/v2` (static-verification.md §6.2/§6.3,
//! PLAN-VERIFY-3 P7.2).
//!
//! A *verdicts file* is the interface an external tool (the harvest, a cached
//! build, a human assumption) uses to tell the compiler which obligations are
//! discharged and with *what trust*. It is **untrusted input** (§7.5):
//! schema-validated, size-capped, and fail-closed — a malformed file is a
//! hard error (E6402 / E6417) that aborts before codegen.
//!
//! v2 (PLAN-VERIFY-3 §6.3, §Q6) adds the trust/method/provenance surface the
//! developer-proof pipeline demands:
//!
//! - `trust` — `proof` (kernel-checked derivation in a recognized port),
//!   `checked` (in-tree deterministic method), `assumed` (human), `open`;
//! - `method` — a **closed registry** (`certificate`, `rederive`,
//!   `descriptor`, `stack-exact`, `interval`): an unknown method is E6417
//!   (fail-closed), never silently tolerated;
//! - file-level `certifier` identity (class/name/recognition/tool/toolchain);
//!   an unrecognized producer's `proof` labels are *downgraded* to `assumed`
//!   with the original claim preserved in `claimed` (§Q6, P2 Q7 rule);
//! - `statement_hash` binding (FR-5): REQUIRED for `certificate`/`rederive`;
//!   the consumer recomputes it and a mismatch is stale ⇒ open (E6421);
//! - `proof_ref` (the v1 dangling promise) is **retired normatively**: v2
//!   never emits it (§3.2 debt item 1).
//!
//! **Parser consolidation (§3.2 debt item 1):** ONE parser parameterized by
//! [`ParseSurface`] — the input-status closure and the echo superset are no
//! longer two functions. The v2 `status`/`trust`/`method` value sets are
//! declared once per surface; the strict reader rejects closed-enum jells on
//! both surfaces.
//!
//! The same schema doubles as the *echo* langc writes beside the artifact
//! (`<Module>.verdicts.inTree.json`): a verdicts file plus the module's
//! `stale_verdicts`, `emitted` accounting, `provably_failing` records, and
//! open reasons (P4/P5) — the added keys are echo-only and skipped by the
//! strict input reader.
//!
//! Hand-rolled JSON — deliberately no `serde` dependency (same
//! `-nodefaultlibs` link constraint as [`crate::codec`]). Fixed key order on
//! the write side (Q11); unknown keys are skipped on the read side (additive
//! schema growth), except the closed enums.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::codec::{push_i64_json, push_str_json};
use crate::model::Obligation;
use crate::semantics::SEMANTICS_VERSION;

/// Schema identifier of verdicts files (`tyu.verdicts/v2`).
pub const VERDICTS_SCHEMA: &str = "tyu.verdicts/v2";

/// The statement-schema the verdicts bind against (§6.2/§6.3).
pub const VERDICTS_STMT: &str = crate::stmt::STMT_SCHEMA;

/// Hard read-side cap for verdicts files (static-verification.md §7.5:
/// `--verdicts` ≤ 4 MiB; over-cap → E6402, fail-closed).
pub const VERDICTS_FILE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Maximum length of an `id`/`id_hash` string (§7.5 — ids are capped so a
/// hostile file cannot turn the verdicts map into a memory sink).
pub const VERDICT_ID_MAX_BYTES: usize = 512;

/// The recognized port certifier registry (verification-trust.md rev 1,
/// §Q11). `language: each entry is a `recognition` id the harvest (and only
/// the harvest) is verified against; a producer outside this set cannot mint
/// `proof` trust — its labels downgrade to `assumed` + `claimed` (§Q6).
pub const RECOGNIZED_CERTIFIERS: &[&str] = &["tyu-port/lean/1"];

/// The parse surface (§3.2 debt item 1): the *input* verdicts file is closed
/// to `discharged`/`assumed`; the *echo* (langc's own output, re-readable as
/// input) also admits `open`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseSurface {
    /// A caller-supplied `--verdicts` file: `status ∈ {discharged, assumed}`.
    Input,
    /// langc's echoes: additionally `open` (the in-tree resolution state).
    Echo,
}

/// A verdict's status. v2: the file surface admits `discharged`/`assumed`/
/// `open` — v2 verdicts are FULL reports (the harvest's `open` records with
/// witnesses are consumed alongside the closed ones; an `open` record closes
/// nothing, it can only cause more checking). The `Echo` surface additionally
/// parses the echo-only members.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerdictStatus {
    Discharged,
    Assumed,
    Open,
}

impl VerdictStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            VerdictStatus::Discharged => "discharged",
            VerdictStatus::Assumed => "assumed",
            VerdictStatus::Open => "open",
        }
    }

    /// The one status parser (consolidated — §3.2 debt item 1): same value
    /// set on both surfaces (v2 files carry open records; the echo adds only
    /// members).
    pub fn parse(s: &str, _surface: ParseSurface) -> Option<VerdictStatus> {
        match s {
            "discharged" => Some(VerdictStatus::Discharged),
            "assumed" => Some(VerdictStatus::Assumed),
            "open" => Some(VerdictStatus::Open),
            _ => None,
        }
    }

    pub const fn is_open(self) -> bool {
        matches!(self, VerdictStatus::Open)
    }

    /// A closed verdict (discharged or assumed) suppresses the runtime check.
    pub const fn is_closed(self) -> bool {
        !self.is_open()
    }
}

/// The trust classes (§Q6, closed set on both surfaces).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trust {
    /// Kernel-checked derivation inside a recognized port.
    Proof,
    /// In-tree deterministic method (theorem-backed in a recognized port).
    Checked,
    /// Human decision, justification recorded.
    Assumed,
    /// Not discharged; the check is retained.
    Open,
}

impl Trust {
    pub const fn as_str(self) -> &'static str {
        match self {
            Trust::Proof => "proof",
            Trust::Checked => "checked",
            Trust::Assumed => "assumed",
            Trust::Open => "open",
        }
    }

    /// Closed on both surfaces: an unknown trust is E6417 (fail-closed).
    pub fn parse(s: &str) -> Option<Trust> {
        match s {
            "proof" => Some(Trust::Proof),
            "checked" => Some(Trust::Checked),
            "assumed" => Some(Trust::Assumed),
            "open" => Some(Trust::Open),
            _ => None,
        }
    }
}

/// The closed method registry (§Q6; the `proven`-admissibility subsets are
/// policy-level, §Q12 — `proven` admits `checked` only from
/// `{descriptor, stack-exact}`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    /// A developer-authored (or fill-generated) proof of a bound statement.
    Certificate,
    /// The port re-runs the in-tree abstract interpreter (P14, T-A/T-B).
    Rederive,
    /// The deterministic descriptor rule (mmio-bounds geometry).
    Descriptor,
    /// The exact in-tree stack analysis (T-C-backed).
    StackExact,
    /// The in-tree interval engine (automation layer; NOT `proven`-admissible
    /// until P14's rederive upgrades it, §Q12).
    Interval,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Certificate => "certificate",
            Method::Rederive => "rederive",
            Method::Descriptor => "descriptor",
            Method::StackExact => "stack-exact",
            Method::Interval => "interval",
        }
    }

    /// Closed: an unknown method is E6417 (fail-closed, §6.3).
    pub fn parse(s: &str) -> Option<Method> {
        match s {
            "certificate" => Some(Method::Certificate),
            "rederive" => Some(Method::Rederive),
            "descriptor" => Some(Method::Descriptor),
            "stack-exact" => Some(Method::StackExact),
            "interval" => Some(Method::Interval),
            _ => None,
        }
    }

    /// `true` when the method binds a `statement_hash` (FR-5, §6.3).
    pub const fn binds_statement(self) -> bool {
        matches!(self, Method::Certificate | Method::Rederive)
    }

    /// `true` when the method is `proven`-admissible for `checked` trust
    /// (§Q12: only the exact-method registry).
    pub const fn proven_admissible_checked(self) -> bool {
        matches!(self, Method::Descriptor | Method::StackExact)
    }
}

/// The proof surface a `certificate` rests on (§Q2): source-level semantics
/// (composition with T-S) or IR semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofSurface {
    Source,
    Ir,
}

impl ProofSurface {
    pub const fn as_str(self) -> &'static str {
        match self {
            ProofSurface::Source => "source",
            ProofSurface::Ir => "ir",
        }
    }

    pub fn parse(s: &str) -> Option<ProofSurface> {
        match s {
            "source" => Some(ProofSurface::Source),
            "ir" => Some(ProofSurface::Ir),
            _ => None,
        }
    }
}

/// The authorship flag of a certificate (§Q10): a never-reviewed fill
/// candidate is `Candidate` and the report/package MUST flag it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authored {
    Developer,
    Candidate,
}

impl Authored {
    pub const fn as_str(self) -> &'static str {
        match self {
            Authored::Developer => "developer",
            Authored::Candidate => "candidate",
        }
    }

    pub fn parse(s: &str) -> Option<Authored> {
        match s {
            "developer" => Some(Authored::Developer),
            "candidate" => Some(Authored::Candidate),
            _ => None,
        }
    }
}

/// The `proof` object kinds (§6.3, closed): `certificate` (a named theorem of
/// a bound statement), `exact` (an in-tree exact method), `rederive` (the
/// port re-run of the abstract interpreter).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofKind {
    Certificate,
    Exact,
    Rederive,
}

impl ProofKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ProofKind::Certificate => "certificate",
            ProofKind::Exact => "exact",
            ProofKind::Rederive => "rederive",
        }
    }

    /// Closed: an unknown proof.kind is E6417.
    pub fn parse(s: &str) -> Option<ProofKind> {
        match s {
            "certificate" => Some(ProofKind::Certificate),
            "exact" => Some(ProofKind::Exact),
            "rederive" => Some(ProofKind::Rederive),
            _ => None,
        }
    }
}

/// Tool/product identity (`tool` in §6.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolIdentity {
    pub name: String,
    pub version: String,
}

/// The file-level certifier identity (§6.3): REQUIRED on harvest-produced
/// files; the producer-recognition gate (§Q6) keys on `recognition`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Certifier {
    pub class: String,
    pub name: String,
    pub recognition: String,
    pub tool: ToolIdentity,
    pub toolchain: String,
}

/// The `proof` object of a record (`proof.kind` closed).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofInfo {
    pub kind: ProofKind,
    /// The statement schema (`tyu.stmt/1.0`), the registry theorem
    /// (`T-C`/`T-A`/…), or `"in-tree"` for a descriptor/interval exact.
    pub statement: String,
    /// `certificate`: the theorem name in the kernel environment.
    pub theorem: Option<String>,
    /// `certificate`: the kernel-check mechanism (`lean-kernel+lean4checker`).
    pub kernel_check: Option<String>,
    /// `certificate`: the developer proof file this theorem lives in.
    pub file: Option<String>,
}

/// The `claimed` member (§Q6 downgrade rule): what an *unrecognized* producer
/// claimed, preserved verbatim so the pending verdict is honest about it.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct Claimed {
    pub status: Option<String>,
    pub trust: Option<String>,
    pub method: Option<String>,
    pub statement_hash: Option<String>,
}

/// One v2 verdict record (§6.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerdictRecord {
    pub id: String,
    pub id_hash: String,
    pub status: VerdictStatus,
    pub trust: Trust,
    pub method: Option<Method>,
    /// `certificate`-only provenance (§Q2): source/ir.
    pub surface: Option<ProofSurface>,
    /// Required iff `method ∈ {certificate, rederive}` (FR-5).
    pub statement_hash: Option<String>,
    /// `certificate`-only authorship (§Q10).
    pub authored: Option<Authored>,
    /// Required iff `trust = proof`.
    pub proof: Option<ProofInfo>,
    /// Unrecognized-producer downgrade record (§Q6).
    pub claimed: Option<Claimed>,
    /// `assumed`: the human justification.
    pub justification: Option<String>,
    /// `open`: the witness reason (`model-unavailable`, `assumption-unresolved`, …).
    pub witness_reason: Option<String>,
    /// Any consumer-facing note.
    pub note: Option<String>,
}

impl VerdictRecord {
    pub fn discharged(id: String, id_hash: String, trust: Trust, method: Method) -> Self {
        Self {
            id,
            id_hash,
            status: VerdictStatus::Discharged,
            trust,
            method: Some(method),
            surface: None,
            statement_hash: None,
            authored: None,
            proof: None,
            claimed: None,
            justification: None,
            witness_reason: None,
            note: None,
        }
    }

    /// The §Q6 downgrade: a producer the consumer does not recognize cannot
    /// mint `proof`/`checked` trust — the record becomes `assumed` with its
    /// claims preserved in `claimed` (P2 Q7 rule, inherited verbatim).
    pub fn downgraded_to_assumed(mut self) -> Self {
        self.claimed = Some(Claimed {
            status: Some(self.status.as_str().to_string()),
            trust: Some(self.trust.as_str().to_string()),
            method: self.method.map(|m| m.as_str().to_string()),
            statement_hash: self.statement_hash.clone(),
        });
        self.status = VerdictStatus::Assumed;
        self.trust = Trust::Assumed;
        self
    }
}

/// The parsed and schema-validated verdicts document (§6.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verdicts {
    pub semantics: String,
    pub stmt: String,
    /// File-level certifier identity (harvest files carry it; an echo may
    /// omit it — readers fill `None`).
    pub certifier: Option<Certifier>,
    pub target: String,
    pub model_semantics: String,
    pub records: Vec<VerdictRecord>,
}

impl Verdicts {
    /// The verdict record for an obligation's computed `(id, id_hash)`, or
    /// `None` when the file carries no matching record — either absent or
    /// hash-mismatched (both are fail-closed: the site stays open and the
    /// record is counted stale by [`Verdicts::stale_count`]).
    pub fn lookup(&self, id: &str, id_hash: &str) -> Option<&VerdictRecord> {
        self.records
            .iter()
            .find(|r| r.id == id && r.id_hash == id_hash)
    }

    /// Number of records that match no obligation's `(id, id_hash)` — stale
    /// verdicts (Q3): renamed/removed obligations, or hash collisions that
    /// failed closed. Counted in the echo and surfaced in the report
    /// (`stale_verdicts`, FR-15).
    pub fn stale_count(&self, obligations: &[Obligation]) -> u32 {
        self.records
            .iter()
            .filter(|r| {
                !obligations
                    .iter()
                    .any(|o| o.id == r.id && o.id_hash == r.id_hash)
            })
            .count() as u32
    }

    /// Whether the file's producer is a recognized port certifier (§Q6).
    /// `None` certifier ⇒ unrecognized (nothing can certify `proof`).
    pub fn producer_recognized(&self) -> bool {
        match &self.certifier {
            Some(c) => RECOGNIZED_CERTIFIERS.contains(&c.recognition.as_str()),
            None => false,
        }
    }

    /// The §Q6 downgrade applied to the whole file when its producer is not
    /// recognized: every `proof`-class record (a kernel-checked certificate —
    /// the ONE class that requires a recognized producer, §Q11) becomes
    /// `assumed` with its claim preserved. `checked` records are the
    /// toolchain's own class (the in-tree determinism survives the round-trip
    /// of langc's own echo). Returns `self` unchanged when recognized.
    pub fn restrict_to_recognized(self) -> Verdicts {
        if self.producer_recognized() {
            return self;
        }
        let records = self
            .records
            .into_iter()
            .map(|r| match r.trust {
                Trust::Proof => r.downgraded_to_assumed(),
                _ => r,
            })
            .collect();
        Verdicts { records, ..self }
    }
}

/// Verdict-file validation failures (fail-closed E6402/E6417, §7.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerdictError {
    /// Not parseable as the artifact shape at all / required-member missing.
    Malformed,
    /// Top-level `schema` is not `tyu.verdicts/v2`.
    SchemaVersion { found: String },
    /// Top-level `semantics` disagrees with `SEMANTICS_VERSION` — caches and
    /// proofs are keyed on it (Q3).
    SemanticsMismatch { found: String },
    /// A closed-enum member (`method`/`proof.kind`/`trust`) is unknown —
    /// E6417, never silently tolerated (§6.3).
    UnknownMethod { found: String, kind: &'static str },
    /// Exceeds the 4 MiB read-side cap.
    TooLarge { size: usize },
}

impl VerdictError {
    /// The diagnostic code these failures surface as (E6402, fail-closed),
    /// except the closed-registry violations (E6417).
    pub fn code(&self) -> u32 {
        match self {
            VerdictError::SchemaVersion { .. }
            | VerdictError::SemanticsMismatch { .. }
            | VerdictError::Malformed
            | VerdictError::TooLarge { .. } => 6402,
            VerdictError::UnknownMethod { .. } => 6417,
        }
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Serialize a v2 verdicts/echo document. `records` are emitted in order;
/// missing top-level optional keys (`tool` omitted when `tool_name` is empty,
/// `stale_verdicts`/`emitted` omitted when zero) are deterministic — the
/// reader fills defaults.
#[allow(clippy::too_many_arguments)]
pub fn encode_verdicts(
    tool_name: &str,
    tool_version: &str,
    certifier: Option<&Certifier>,
    target: &str,
    model_semantics: &str,
    records: &[VerdictRecord],
    stale_verdicts: u32,
    emitted: &EmittedChecksData,
) -> Result<Vec<u8>, VerdictError> {
    // The legacy tool identity params are retained for API stability; the
    // file-level `certifier` object carries the identity in v2.
    let _ = (tool_name, tool_version);
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(b"{\"schema\":");
    push_str_json(&mut out, VERDICTS_SCHEMA);
    out.extend_from_slice(b",\"certifier\":");
    match certifier {
        Some(c) => {
            out.extend_from_slice(b"{\"class\":");
            push_str_json(&mut out, &c.class);
            out.extend_from_slice(b",\"name\":");
            push_str_json(&mut out, &c.name);
            out.extend_from_slice(b",\"recognition\":");
            push_str_json(&mut out, &c.recognition);
            out.extend_from_slice(b",\"tool\":{\"name\":");
            push_str_json(&mut out, &c.tool.name);
            out.extend_from_slice(b",\"version\":");
            push_str_json(&mut out, &c.tool.version);
            out.extend_from_slice(b"},\"toolchain\":");
            push_str_json(&mut out, &c.toolchain);
            out.push(b'}');
        }
        None => out.extend_from_slice(b"null"),
    }
    out.extend_from_slice(b",\"semantics\":");
    push_str_json(&mut out, SEMANTICS_VERSION);
    out.extend_from_slice(b",\"stmt\":");
    push_str_json(&mut out, VERDICTS_STMT);
    out.extend_from_slice(b",\"target\":");
    push_str_json(&mut out, target);
    out.extend_from_slice(b",\"model_semantics\":");
    push_str_json(&mut out, model_semantics);
    out.extend_from_slice(b",\"verdicts\":[");
    for (i, r) in records.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        push_record_json(&mut out, r);
    }
    out.extend_from_slice(b"]");
    if stale_verdicts != 0 {
        out.extend_from_slice(b",\"stale_verdicts\":");
        push_i64_json(&mut out, stale_verdicts as i64);
    }
    if emitted.subtype_range != 0 || emitted.contract != 0 || emitted.mmio_bounds != 0 {
        out.extend_from_slice(b",\"emitted\":{\"subtype_range\":");
        push_i64_json(&mut out, emitted.subtype_range as i64);
        out.extend_from_slice(b",\"contract\":");
        push_i64_json(&mut out, emitted.contract as i64);
        out.extend_from_slice(b",\"mmio_bounds\":");
        push_i64_json(&mut out, emitted.mmio_bounds as i64);
        out.push(b'}');
    }
    out.push(b'}');
    if out.len() > VERDICTS_FILE_MAX_BYTES {
        return Err(VerdictError::TooLarge { size: out.len() });
    }
    Ok(out)
}

fn push_record_json(out: &mut Vec<u8>, r: &VerdictRecord) {
    out.extend_from_slice(b"{\"id\":");
    push_str_json(out, &r.id);
    out.extend_from_slice(b",\"id_hash\":");
    push_str_json(out, &r.id_hash);
    out.extend_from_slice(b",\"status\":");
    push_str_json(out, r.status.as_str());
    out.extend_from_slice(b",\"trust\":");
    push_str_json(out, r.trust.as_str());
    if let Some(m) = &r.method {
        out.extend_from_slice(b",\"method\":");
        push_str_json(out, m.as_str());
    }
    if let Some(s) = &r.surface {
        out.extend_from_slice(b",\"surface\":");
        push_str_json(out, s.as_str());
    }
    if let Some(h) = &r.statement_hash {
        out.extend_from_slice(b",\"statement_hash\":");
        push_str_json(out, h);
    }
    if let Some(a) = &r.authored {
        out.extend_from_slice(b",\"authored\":");
        push_str_json(out, a.as_str());
    }
    if let Some(p) = &r.proof {
        out.extend_from_slice(b",\"proof\":{");
        out.extend_from_slice(b"\"kind\":");
        push_str_json(out, p.kind.as_str());
        out.extend_from_slice(b",\"statement\":");
        push_str_json(out, &p.statement);
        if let Some(t) = &p.theorem {
            out.extend_from_slice(b",\"theorem\":");
            push_str_json(out, t);
        }
        if let Some(k) = &p.kernel_check {
            out.extend_from_slice(b",\"kernel_check\":");
            push_str_json(out, k);
        }
        if let Some(f) = &p.file {
            out.extend_from_slice(b",\"file\":");
            push_str_json(out, f);
        }
        out.push(b'}');
    }
    if let Some(c) = &r.claimed {
        out.extend_from_slice(b",\"claimed\":{\"status\":");
        match &c.status {
            Some(v) => push_str_json(out, v),
            None => out.extend_from_slice(b"null"),
        }
        out.extend_from_slice(b",\"trust\":");
        match &c.trust {
            Some(v) => push_str_json(out, v),
            None => out.extend_from_slice(b"null"),
        }
        out.extend_from_slice(b",\"method\":");
        match &c.method {
            Some(v) => push_str_json(out, v),
            None => out.extend_from_slice(b"null"),
        }
        out.extend_from_slice(b",\"statement_hash\":");
        match &c.statement_hash {
            Some(v) => push_str_json(out, v),
            None => out.extend_from_slice(b"null"),
        }
        out.push(b'}');
    }
    if let Some(j) = &r.justification {
        out.extend_from_slice(b",\"justification\":");
        push_str_json(out, j);
    }
    if let Some(w) = &r.witness_reason {
        out.extend_from_slice(b",\"witness\":{\"reason\":");
        push_str_json(out, w);
        out.push(b'}');
    }
    if let Some(n) = &r.note {
        out.extend_from_slice(b",\"note\":");
        push_str_json(out, n);
    }
    out.push(b'}');
}

/// Serialize a langc *echo* (the superset document `<Module>.verdicts.inTree.
/// json`): a v2 verdicts file plus the module's `stale_verdicts`, `emitted`
/// accounting, `provably_failing` records, and open reasons (P4/P5). The
/// result is itself a valid `--verdicts` input — the added keys are echo-only
/// and skipped by the strict input reader.
#[allow(clippy::too_many_arguments)]
pub fn encode_echo(
    tool_name: &str,
    tool_version: &str,
    target: &str,
    model_semantics: &str,
    records: &[VerdictRecord],
    stale_verdicts: u32,
    emitted: &EmittedChecksData,
    provably_failing: &[ProvablyFailingRecord],
    open_reasons: &[OpenReasonRecord],
    in_tree_verdicts: u32,
    verify_tool: Option<&str>,
) -> Result<Vec<u8>, VerdictError> {
    let certifier = Certifier {
        class: "in-tree".to_string(),
        name: tool_name.to_string(),
        recognition: "-".to_string(),
        tool: ToolIdentity {
            name: tool_name.to_string(),
            version: tool_version.to_string(),
        },
        toolchain: "-".to_string(),
    };
    let doc = encode_verdicts(
        tool_name,
        tool_version,
        Some(&certifier),
        target,
        model_semantics,
        records,
        stale_verdicts,
        emitted,
    )?;
    // The doc ends with the document `}`; splice the echo-only members in
    // before it (the members themselves are terminated by that brace).
    let body = &doc[..doc.len() - 1];
    let mut out = Vec::with_capacity(doc.len() + 512);
    out.extend_from_slice(body);
    // Echo-only members — the reader understands them; the strict input path
    // skips them as unknown keys.
    if !provably_failing.is_empty() {
        out.extend_from_slice(b",\"provably_failing\":[");
        for (i, p) in provably_failing.iter().enumerate() {
            if i != 0 {
                out.push(b',');
            }
            out.extend_from_slice(b"{\"id\":");
            push_str_json(&mut out, &p.id);
            out.extend_from_slice(b",\"note\":");
            push_str_json(&mut out, &p.note);
            out.push(b'}');
        }
        out.push(b']');
    }
    if !open_reasons.is_empty() {
        out.extend_from_slice(b",\"open_reasons\":[");
        for (i, r) in open_reasons.iter().enumerate() {
            if i != 0 {
                out.push(b',');
            }
            out.extend_from_slice(b"{\"id\":");
            push_str_json(&mut out, &r.id);
            out.extend_from_slice(b",\"reason\":");
            push_str_json(&mut out, &r.reason);
            out.push(b'}');
        }
        out.push(b']');
    }
    if !records.is_empty() {
        // The source split of the closed verdicts: `in_tree` passed in, the
        // file-sourced remainder is `records - in_tree`.
        let file = records.len() as u32 - core::cmp::min(records.len() as u32, in_tree_verdicts);
        out.extend_from_slice(b",\"verdict_sources\":{\"file\":");
        push_i64_json(&mut out, file as i64);
        out.extend_from_slice(b",\"in_tree\":");
        push_i64_json(&mut out, in_tree_verdicts as i64);
        out.push(b'}');
    }
    if let Some(tool) = verify_tool {
        out.extend_from_slice(b",\"verify_tool\":");
        push_str_json(&mut out, tool);
    }
    out.push(b'}');
    if out.len() > VERDICTS_FILE_MAX_BYTES {
        return Err(VerdictError::TooLarge { size: out.len() });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Parse and schema-validate a verdicts file (input surface). Fail-closed: a
/// wrong schema version, a semantics-version mismatch, an oversized or
/// malformed file is an error (E6402/E6417), never a silent best-effort read.
pub fn read_verdicts(bytes: &[u8]) -> Result<Verdicts, VerdictError> {
    let mut e = Echo::read(bytes, ParseSurface::Input)?;
    e.verdicts.records.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(e.verdicts)
}

/// Parse a langc verdicts echo (`<Module>.verdicts.inTree.json`), which is a
/// verdicts file plus the module's `stale_verdicts` count and `emitted`
/// check-accounting (FR-15). Same fail-closed validation as
/// [`read_verdicts`], on the echo surface (admits `open`); the echo-only
/// keys default to zero when absent, so an ordinary input file parses too.
pub fn read_echo(bytes: &[u8]) -> Result<Echo, VerdictError> {
    Echo::read(bytes, ParseSurface::Echo)
}

/// A `provably_failing` echo/report record (slice P5): the in-tree interval
/// engine proved the site's value is *always* outside the target range — the
/// check is retained; the site is reported for diagnosis (never discharged).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvablyFailingRecord {
    pub id: String,
    /// Human-readable note (the interval reason).
    pub note: String,
}

/// An open-reason record (slice P5, FR-18 quality bar): why an open
/// obligation is not discharged (e.g. `"value interval <top> ..."`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenReasonRecord {
    pub id: String,
    pub reason: String,
}

/// The parsed verdicts echo (verdicts + stale count + emitted accounting +
/// provably-failing records + open reasons + discharge-source counts).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Echo {
    pub verdicts: Verdicts,
    pub stale_verdicts: u32,
    pub emitted: EmittedChecksData,
    pub provably_failing: Vec<ProvablyFailingRecord>,
    pub open_reasons: Vec<OpenReasonRecord>,
    /// Discharge-source counts (`verdict_sources`, slice P5): closed verdicts
    /// decided by the verdicts file vs the in-tree dischargers.
    pub file_verdicts: u32,
    pub in_tree_verdicts: u32,
}

/// The per-module `emitted` accounting an echo carries (FR-15).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct EmittedChecksData {
    /// Emitted subtype-range checks (C1/C2/C3) — equals the open subtype-site
    /// count: every undischarged/unassumed site emits.
    pub subtype_range: u32,
    /// Emitted contract checks (C5/C6 needs/ensures traps).
    pub contract: u32,
    /// Emitted emulated-aperture MMIO bounds checks (C7). Per-word granularity
    /// (Q8): a word with any open access retains all its checks.
    pub mmio_bounds: u32,
}

impl Echo {
    /// The one parser (consolidated, §3.2 debt item 1): a strict schema-valid
    /// reader over the given surface. The v2 closed enums (`status` per
    /// surface, `trust`, `method`, `proof.kind`) are declared once here.
    fn read(bytes: &[u8], surface: ParseSurface) -> Result<Echo, VerdictError> {
        if bytes.len() > VERDICTS_FILE_MAX_BYTES {
            return Err(VerdictError::TooLarge { size: bytes.len() });
        }
        let mut r = VReader { b: bytes, i: 0 };
        let doc = r.parse_doc(surface)?;
        if doc.schema != VERDICTS_SCHEMA {
            return Err(VerdictError::SchemaVersion { found: doc.schema });
        }
        if doc.semantics != SEMANTICS_VERSION {
            return Err(VerdictError::SemanticsMismatch {
                found: doc.semantics,
            });
        }
        Ok(Echo {
            verdicts: doc.verdicts,
            stale_verdicts: doc.stale_verdicts,
            emitted: doc.emitted,
            provably_failing: doc.provably_failing,
            open_reasons: doc.open_reasons,
            file_verdicts: doc.file_verdicts,
            in_tree_verdicts: doc.in_tree_verdicts,
        })
    }
}

struct VReader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> VReader<'a> {
    fn err<T>(&self) -> Result<T, VerdictError> {
        Err(VerdictError::Malformed)
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn bump(&mut self) -> Result<u8, VerdictError> {
        let b = self.b.get(self.i).copied().ok_or(VerdictError::Malformed)?;
        self.i += 1;
        Ok(b)
    }

    fn expect(&mut self, want: u8) -> Result<(), VerdictError> {
        self.skip_ws();
        if self.bump()? != want {
            self.err()
        } else {
            Ok(())
        }
    }

    fn parse_doc(&mut self, surface: ParseSurface) -> Result<Doc, VerdictError> {
        self.skip_ws();
        if self.bump()? != b'{' {
            return self.err();
        }
        let mut schema: Option<String> = None;
        let mut semantics: Option<String> = None;
        let mut stmt: Option<String> = None;
        let mut certifier: Option<Certifier> = None;
        let mut target: Option<String> = None;
        let mut model_semantics: Option<String> = None;
        let mut records: Option<Vec<VerdictRecord>> = None;
        let mut stale_verdicts: Option<u32> = None;
        let mut emitted: Option<EmittedChecksData> = None;
        let mut provably_failing: Option<Vec<ProvablyFailingRecord>> = None;
        let mut open_reasons: Option<Vec<OpenReasonRecord>> = None;
        let mut file_verdicts: Option<u32> = None;
        let mut in_tree_verdicts: Option<u32> = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "schema" => schema = Some(self.parse_string()?),
                "semantics" => semantics = Some(self.parse_string()?),
                "stmt" => stmt = Some(self.parse_string()?),
                "certifier" => certifier = self.parse_certifier()?,
                "target" => target = Some(self.parse_string()?),
                "model_semantics" => model_semantics = Some(self.parse_string()?),
                "verdicts" => records = Some(self.parse_records(surface)?),
                // Echo extras (P4/P5): parsed here so tyu's report composition
                // can read them back; the strict `read_verdicts` input path
                // skips them as unknown keys.
                "stale_verdicts" => stale_verdicts = Some(self.parse_u32()?),
                "emitted" => emitted = Some(self.parse_emitted()?),
                "provably_failing" => provably_failing = Some(self.parse_pfi()?),
                "open_reasons" => open_reasons = Some(self.parse_reasons()?),
                "verdict_sources" => {
                    let (f, t) = self.parse_verdict_sources()?;
                    file_verdicts = Some(f);
                    in_tree_verdicts = Some(t);
                }
                // Additive keys are skipped — readers tolerate future growth.
                _ => self.skip_value()?,
            }
        }
        let (Some(schema), Some(semantics), Some(records)) = (schema, semantics, records) else {
            return self.err();
        };
        Ok(Doc {
            schema,
            semantics: semantics.clone(),
            verdicts: Verdicts {
                semantics,
                stmt: stmt.unwrap_or_else(|| VERDICTS_STMT.to_string()),
                certifier,
                target: target.unwrap_or_default(),
                model_semantics: model_semantics.unwrap_or_default(),
                records,
            },
            stale_verdicts: stale_verdicts.unwrap_or(0),
            emitted: emitted.unwrap_or_default(),
            provably_failing: provably_failing.unwrap_or_default(),
            open_reasons: open_reasons.unwrap_or_default(),
            file_verdicts: file_verdicts.unwrap_or(0),
            in_tree_verdicts: in_tree_verdicts.unwrap_or(0),
        })
    }

    fn parse_certifier(&mut self) -> Result<Option<Certifier>, VerdictError> {
        self.skip_ws();
        if self.peek() == Some(b'n') {
            // `null`
            return self.skip_value().map(|_| None);
        }
        self.expect(b'{')?;
        let mut class: Option<String> = None;
        let mut name: Option<String> = None;
        let mut recognition: Option<String> = None;
        let mut tool_name: Option<String> = None;
        let mut tool_version: Option<String> = None;
        let mut toolchain: Option<String> = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "class" => class = Some(self.parse_string()?),
                "name" => name = Some(self.parse_string()?),
                "recognition" => recognition = Some(self.parse_string()?),
                "tool" => {
                    self.expect(b'{')?;
                    loop {
                        self.skip_ws();
                        match self.peek() {
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            Some(b',') => {
                                self.i += 1;
                            }
                            _ => {}
                        }
                        self.skip_ws();
                        let k = self.parse_string()?;
                        self.expect(b':')?;
                        match k.as_str() {
                            "name" => tool_name = Some(self.parse_string()?),
                            "version" => tool_version = Some(self.parse_string()?),
                            _ => self.skip_value()?,
                        }
                    }
                }
                "toolchain" => toolchain = Some(self.parse_string()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(class), Some(name), Some(recognition)) = (class, name, recognition) else {
            return self.err();
        };
        Ok(Some(Certifier {
            class,
            name,
            recognition,
            tool: ToolIdentity {
                name: tool_name.unwrap_or_default(),
                version: tool_version.unwrap_or_default(),
            },
            toolchain: toolchain.unwrap_or_default(),
        }))
    }

    fn parse_records(&mut self, surface: ParseSurface) -> Result<Vec<VerdictRecord>, VerdictError> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                break;
            }
            out.push(self.parse_record(surface)?);
        }
        Ok(out)
    }

    fn parse_record(&mut self, surface: ParseSurface) -> Result<VerdictRecord, VerdictError> {
        self.expect(b'{')?;
        let mut id: Option<String> = None;
        let mut id_hash: Option<String> = None;
        let mut status: Option<VerdictStatus> = None;
        let mut trust: Option<Trust> = None;
        let mut method: Option<Method> = None;
        let mut surface_field: Option<ProofSurface> = None;
        let mut statement_hash: Option<String> = None;
        let mut authored: Option<Authored> = None;
        let mut proof: Option<ProofInfo> = None;
        let mut claimed: Option<Claimed> = None;
        let mut justification: Option<String> = None;
        let mut witness_reason: Option<String> = None;
        let mut note: Option<String> = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "id" => id = Some(self.parse_string()?),
                "id_hash" => id_hash = Some(self.parse_string()?),
                "status" => {
                    let v = self.parse_string()?;
                    status =
                        Some(VerdictStatus::parse(&v, surface).ok_or(VerdictError::Malformed)?);
                }
                "trust" => {
                    let v = self.parse_string()?;
                    trust = Some(Trust::parse(&v).ok_or(VerdictError::Malformed)?);
                }
                "method" => {
                    let v = self.parse_string()?;
                    method =
                        Some(
                            Method::parse(&v).ok_or_else(|| VerdictError::UnknownMethod {
                                found: v.clone(),
                                kind: "method",
                            })?,
                        );
                }
                "surface" => {
                    let v = self.parse_string()?;
                    surface_field = Some(ProofSurface::parse(&v).ok_or(VerdictError::Malformed)?);
                }
                "statement_hash" => statement_hash = Some(self.parse_string()?),
                "authored" => {
                    let v = self.parse_string()?;
                    authored = Some(Authored::parse(&v).ok_or(VerdictError::Malformed)?);
                }
                "proof" => proof = Some(self.parse_proof()?),
                "claimed" => claimed = Some(self.parse_claimed()?),
                "justification" => justification = Some(self.parse_string()?),
                "witness" => {
                    self.expect(b'{')?;
                    loop {
                        self.skip_ws();
                        match self.peek() {
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            Some(b',') => {
                                self.i += 1;
                            }
                            _ => {}
                        }
                        self.skip_ws();
                        let k = self.parse_string()?;
                        self.expect(b':')?;
                        if k == "reason" {
                            witness_reason = Some(self.parse_string()?);
                        } else {
                            self.skip_value()?;
                        }
                    }
                }
                "note" => note = Some(self.parse_string()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(id), Some(id_hash), Some(status)) = (id, id_hash, status) else {
            return self.err();
        };
        // §6.3 rules: trust required on discharged; proof required iff
        // trust=proof; statement_hash required for certificate/rederive.
        let trust = trust.unwrap_or(Trust::Open);
        if status.is_closed() && trust == Trust::Open {
            return self.err();
        }
        if trust == Trust::Proof && proof.is_none() {
            return self.err();
        }
        if let Some(m) = method {
            if m.binds_statement() && statement_hash.is_none() {
                return self.err();
            }
        }
        Ok(VerdictRecord {
            id,
            id_hash,
            status,
            trust,
            method,
            surface: surface_field,
            statement_hash,
            authored,
            proof,
            claimed,
            justification,
            witness_reason,
            note,
        })
    }

    fn parse_proof(&mut self) -> Result<ProofInfo, VerdictError> {
        self.expect(b'{')?;
        let mut kind: Option<ProofKind> = None;
        let mut statement: Option<String> = None;
        let mut theorem: Option<String> = None;
        let mut kernel_check: Option<String> = None;
        let mut file: Option<String> = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "kind" => {
                    let v = self.parse_string()?;
                    kind =
                        Some(
                            ProofKind::parse(&v).ok_or_else(|| VerdictError::UnknownMethod {
                                found: v.clone(),
                                kind: "proof.kind",
                            })?,
                        );
                }
                "statement" => statement = Some(self.parse_string()?),
                "theorem" => theorem = Some(self.parse_string()?),
                "kernel_check" => kernel_check = Some(self.parse_string()?),
                "file" => file = Some(self.parse_string()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(kind), Some(statement)) = (kind, statement) else {
            return self.err();
        };
        Ok(ProofInfo {
            kind,
            statement,
            theorem,
            kernel_check,
            file,
        })
    }

    fn parse_claimed(&mut self) -> Result<Claimed, VerdictError> {
        self.expect(b'{')?;
        let mut claimed = Claimed::default();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "status" => claimed.status = Some(self.parse_string()?),
                "trust" => claimed.trust = Some(self.parse_string()?),
                "method" => claimed.method = Some(self.parse_string()?),
                "statement_hash" => claimed.statement_hash = Some(self.parse_string()?),
                _ => self.skip_value()?,
            }
        }
        Ok(claimed)
    }

    fn parse_verdict_sources(&mut self) -> Result<(u32, u32), VerdictError> {
        self.expect(b'{')?;
        let mut file = 0u32;
        let mut in_tree = 0u32;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "file" => file = self.parse_u32()?,
                "in_tree" => in_tree = self.parse_u32()?,
                _ => self.skip_value()?,
            }
        }
        Ok((file, in_tree))
    }

    fn parse_pfi(&mut self) -> Result<Vec<ProvablyFailingRecord>, VerdictError> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                break;
            }
            self.expect(b'{')?;
            let mut id: Option<String> = None;
            let mut note: Option<String> = None;
            loop {
                self.skip_ws();
                match self.peek() {
                    Some(b'}') => {
                        self.i += 1;
                        break;
                    }
                    Some(b',') => {
                        self.i += 1;
                    }
                    _ => {}
                }
                self.skip_ws();
                let key = self.parse_string()?;
                self.expect(b':')?;
                match key.as_str() {
                    "id" => id = Some(self.parse_string()?),
                    "note" => note = Some(self.parse_string()?),
                    _ => self.skip_value()?,
                }
            }
            let (Some(id), Some(note)) = (id, note) else {
                return self.err();
            };
            out.push(ProvablyFailingRecord { id, note });
        }
        Ok(out)
    }

    fn parse_reasons(&mut self) -> Result<Vec<OpenReasonRecord>, VerdictError> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                break;
            }
            self.expect(b'{')?;
            let mut id: Option<String> = None;
            let mut reason: Option<String> = None;
            loop {
                self.skip_ws();
                match self.peek() {
                    Some(b'}') => {
                        self.i += 1;
                        break;
                    }
                    Some(b',') => {
                        self.i += 1;
                    }
                    _ => {}
                }
                self.skip_ws();
                let key = self.parse_string()?;
                self.expect(b':')?;
                match key.as_str() {
                    "id" => id = Some(self.parse_string()?),
                    "reason" => reason = Some(self.parse_string()?),
                    _ => self.skip_value()?,
                }
            }
            let (Some(id), Some(reason)) = (id, reason) else {
                return self.err();
            };
            out.push(OpenReasonRecord { id, reason });
        }
        Ok(out)
    }

    fn parse_emitted(&mut self) -> Result<EmittedChecksData, VerdictError> {
        self.expect(b'{')?;
        let mut data = EmittedChecksData::default();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "subtype_range" => data.subtype_range = self.parse_u32()?,
                "contract" => data.contract = self.parse_u32()?,
                "mmio_bounds" => data.mmio_bounds = self.parse_u32()?,
                _ => self.skip_value()?,
            }
        }
        Ok(data)
    }

    fn parse_u32(&mut self) -> Result<u32, VerdictError> {
        let v = self.parse_i64()?;
        if v < 0 || v > u32::MAX as i64 {
            return self.err();
        }
        Ok(v as u32)
    }

    fn parse_string(&mut self) -> Result<String, VerdictError> {
        self.skip_ws();
        if self.bump()? != b'"' {
            return self.err();
        }
        let mut out = Vec::new();
        loop {
            let Some(&b) = self.b.get(self.i) else {
                return self.err();
            };
            self.i += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else {
                        return self.err();
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => return self.err(),
                        _ => return self.err(),
                    }
                }
                // JSON forbids raw control characters inside strings.
                0..=31 => return self.err(),
                _ => out.push(b),
            }
            // §7.5: id/id_hash strings are capped so a hostile file cannot
            // turn the verdicts map into a memory sink.
            if out.len() > VERDICT_ID_MAX_BYTES {
                return self.err();
            }
        }
        String::from_utf8(out).map_err(|_| VerdictError::Malformed)
    }

    fn parse_i64(&mut self) -> Result<i64, VerdictError> {
        self.skip_ws();
        let mut neg = false;
        if self.peek() == Some(b'-') {
            neg = true;
            self.i += 1;
        }
        let start = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        if self.i == start {
            return self.err();
        }
        let digits =
            core::str::from_utf8(&self.b[start..self.i]).map_err(|_| VerdictError::Malformed)?;
        let v: i64 = digits.parse().map_err(|_| VerdictError::Malformed)?;
        Ok(if neg { -v } else { v })
    }

    fn skip_value(&mut self) -> Result<(), VerdictError> {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => {
                self.parse_string()?;
            }
            Some(b'{') => {
                self.bump()?;
                let mut depth = 1usize;
                while depth > 0 {
                    match self.peek() {
                        Some(b'{') => depth += 1,
                        Some(b'}') => depth -= 1,
                        Some(b'"') => {
                            self.parse_string()?;
                            continue;
                        }
                        None => return self.err(),
                        _ => {}
                    }
                    self.i += 1;
                }
            }
            Some(b'[') => {
                self.bump()?;
                let mut depth = 1usize;
                while depth > 0 {
                    match self.peek() {
                        Some(b'[') => depth += 1,
                        Some(b']') => depth -= 1,
                        Some(b'"') => {
                            self.parse_string()?;
                            continue;
                        }
                        None => return self.err(),
                        _ => {}
                    }
                    self.i += 1;
                }
            }
            Some(b't') => {
                self.skip_token(b"true")?;
            }
            Some(b'f') => {
                self.skip_token(b"false")?;
            }
            Some(b'n') => {
                self.skip_token(b"null")?;
            }
            Some(b'-' | b'0'..=b'9') => {
                self.parse_i64()?;
            }
            _ => return self.err(),
        }
        Ok(())
    }

    fn skip_token(&mut self, tok: &[u8]) -> Result<(), VerdictError> {
        if self.b[self.i..].starts_with(tok) {
            self.i += tok.len();
            Ok(())
        } else {
            self.err()
        }
    }
}

struct Doc {
    schema: String,
    semantics: String,
    verdicts: Verdicts,
    stale_verdicts: u32,
    emitted: EmittedChecksData,
    provably_failing: Vec<ProvablyFailingRecord>,
    open_reasons: Vec<OpenReasonRecord>,
    file_verdicts: u32,
    in_tree_verdicts: u32,
}
