//! The certification package — `<image>.tyucert/` (PLAN-VERIFY-3 §6.6,
//! P11.3, carried from PLAN-VERIFY-2 Q15).
//!
//! A first-class build artifact paired with the shipped image through
//! digests, not by convention. Layout:
//!
//! ```text
//! <image>.tyucert/
//!   cert.json                    # canonical index — identity, bindings, member map
//!   obligations/<Module>.obl.json
//!   verdicts/<Module>.verdicts.json
//!   statements/<Module>.stmt.json
//!   proofs/<Module>/<id_hash>/proof.lean + rerun.json
//!   evidence/{conformance,differential,axiom_audit,candidates}.json
//!   evidence/{toolchain.lock,tcb.json}
//!   package.sig                  # detached HMAC-SHA256 over package_digest
//! ```
//!
//! **Bindings (normative):**
//!
//! - **B1** (package → image): each module entry's `module_digest` is the
//!   SHA-256 of the exact shipped module bytes. `tyu deploy` verifies it
//!   before shipping (FR-21, E6503 on mismatch); an auditor re-checks it with
//!   `tyu cert verify`. This kills the "certificate for some other build"
//!   failure mode.
//! - **B2** (package → signed module): `manifest_digest` equals the
//!   `verdict_digest` inside the module's signed `verify_manifest` record —
//!   the manifest is the anchor, the package is the payload, neither is
//!   meaningful without the other.
//! - **B3** (package → proofs): every member is content-addressed
//!   (`members[]` carries each member's SHA-256 + size); certificate verdict
//!   entries reference their proof material by obligation `id_hash` under
//!   `proofs/<Module>/<id_hash>/`, and that material is itself a member.
//! - **B4** (package integrity): `package_digest` over the canonical index
//!   (digest field excluded); `package.sig` is a detached HMAC-SHA256 (the
//!   `lmod-sign` key surface) over the digest. `tyu cert verify` checks B4
//!   before anything else.
//!
//! **Who consumes it:** `tyu deploy` (FR-21: verify B1/B2 before shipping),
//! `tyu cert verify|show|diff` (host-side verification/inspection), fleet
//! operators and external auditors (self-describing — schemas, registries,
//! the TCB boundary, and re-run instructions travel inside). The device never
//! sees the package (non-goal); it sees only the manifest.
//!
//! Hand-rolled JSON throughout (FR-15); every integrity digest is SHA-256
//! (FR-14); fnv-1a is confined to identity keys by the verifier codecs.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::args::DeployVerifyPolicy;

/// Schema identifiers.
pub const CERT_SCHEMA: &str = "tyu.cert/v1";
/// The index read cap (§7.5 discipline: every new JSON codec is size-capped).
pub const INDEX_MAX_BYTES: usize = 16 * 1024 * 1024;
/// Depth cap for the hand-rolled index parser (hostile-input safety).
const JSON_DEPTH_CAP: usize = 64;
/// Member-path cap.
const MEMBER_PATH_MAX: usize = 512;

const STATEMENTS_DOC_SCHEMA: &str = "tyu.stmt/1.0";
const AXIOM_AUDIT_SCHEMA: &str = "tyu.axiom-audit/1";
const CONFORMANCE_SCHEMA: &str = "tyu.conformance/1";
const DIFFERENTIAL_SCHEMA: &str = "tyu.differential/1";
const TCB_SCHEMA: &str = "tyu.tcb/1";
const CANDIDATES_SCHEMA: &str = "tyu.candidates/1";
const RERUN_SCHEMA: &str = "tyu.rerun/1";

/// E-code for certification-package pairing failures (§6.9: E6503
/// `E_CERT_PAIRING` — B1 module digest / B2 manifest digest mismatch).
pub const E_CERT_PAIRING: u32 = 6503;
/// E-code for a malformed certification index (reader-side, same band).
pub const E_CERT_INDEX_MALFORMED: u32 = 6504;

// ---------------------------------------------------------------------------
// Index model
// ---------------------------------------------------------------------------

/// One `modules[]` entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertModule {
    pub name: String,
    /// B1 — SHA-256 (hex) of the exact shipped module bytes. `None` for a
    /// module with verification state whose bytes are not independently
    /// shipped (a callee linked into the root image).
    pub module_digest: Option<String>,
    /// B2 — the module's signed `verify_manifest` verdict-digest (hex).
    /// `None` when the module carries no manifest in its shipped bytes.
    pub manifest_digest: Option<String>,
    pub obl_digest: String,
    pub verdicts_digest: String,
    pub target: String,
    pub model: String,
    /// counts: proof, checked, assumed, open.
    pub counts: [u32; 4],
    /// candidate-authored certificate ratio, basis 10000 (§Q10).
    pub candidate_ratio: u16,
    /// §Q17 claims: the developer's declared claim surface.
    pub claims: Claims,
}

/// The §Q17 claims record: per-kind obligation counts + authored intents.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct Claims {
    /// `(kind, count)` pairs, sorted by kind.
    pub kinds: Vec<(String, u32)>,
    /// The authored intent labels, sorted + deduped.
    pub intents: Vec<String>,
}

/// One `certifiers[]` entry (a recognized producer that actually certified).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificateId {
    pub class: String,
    pub name: String,
    pub recognition: String,
    pub tool_name: String,
    pub tool_version: String,
    pub toolchain: String,
    /// The methods the producer's verdicts exercised (`certificate`, …).
    pub methods: Vec<String>,
}

/// One `members[]` entry — a content-addressed package member.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

/// A parsed `cert.json` index (the `cert_index_decode` fuzz target's surface).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertIndex {
    pub schema: String,
    pub image_name: String,
    pub image_fingerprint: String,
    pub semantics: String,
    pub stmt: String,
    pub policy: String,
    pub modules: Vec<CertModule>,
    pub certifiers: Vec<CertificateId>,
    pub members: Vec<Member>,
    /// `None` for the digest-excluded canonical form (B4's hash input).
    pub package_digest: Option<String>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Reader-side failures (the fuzz surface's error set).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CertError {
    /// Malformed / over-cap / closed-schema violations.
    Malformed { code: u32, detail: String },
    /// A pairing violation (B1/B2): E6503.
    Pairing { code: u32, detail: String },
}

impl CertError {
    fn malformed(detail: impl Into<String>) -> Self {
        CertError::Malformed {
            code: E_CERT_INDEX_MALFORMED,
            detail: detail.into(),
        }
    }
    pub fn code(&self) -> u32 {
        match self {
            CertError::Malformed { code, .. } | CertError::Pairing { code, .. } => *code,
        }
    }
    pub fn detail(&self) -> &str {
        match self {
            CertError::Malformed { detail, .. } | CertError::Pairing { detail, .. } => detail,
        }
    }
}

impl From<CertError> for crate::error::TyuError {
    fn from(e: CertError) -> Self {
        let code = e.code();
        let detail = e.detail().to_string();
        match e {
            CertError::Pairing { .. } => crate::error::TyuError::VerifyPairing { code, detail },
            CertError::Malformed { .. } => crate::error::TyuError::Cert { code, detail },
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal JSON (hand-rolled, FR-15)
// ---------------------------------------------------------------------------

/// A minimal JSON value (key order preserved → canonical re-serialization).
#[derive(Clone, Debug, Eq, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Parser over UTF-8 bytes; depth-capped, fail-closed, never panics.
fn parse_json(bytes: &[u8]) -> Result<Json, CertError> {
    let s = std::str::from_utf8(bytes).map_err(|_| CertError::malformed("index is not UTF-8"))?;
    let mut p = Parser {
        b: s.as_bytes(),
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(CertError::malformed("trailing bytes after the document"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.b.get(self.i).copied()
    }

    fn value(&mut self) -> Result<Json, CertError> {
        if self.depth >= JSON_DEPTH_CAP {
            return Err(CertError::malformed("nesting too deep"));
        }
        self.depth += 1;
        let v = match self
            .peek()
            .ok_or_else(|| CertError::malformed("unexpected EOF"))?
        {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            other => Err(CertError::malformed(format!(
                "unexpected byte 0x{other:02x} at offset {}",
                self.i
            ))),
        };
        self.depth -= 1;
        v
    }

    fn literal(&mut self, lit: &str, v: Json) -> Result<Json, CertError> {
        let end = self.i + lit.len();
        if end > self.b.len() || &self.b[self.i..end] != lit.as_bytes() {
            return Err(CertError::malformed("malformed literal"));
        }
        self.i = end;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, CertError> {
        // Collects the raw run between escapes, then decodes escapes.
        let mut out = String::new();
        self.i += 1; // opening quote
        loop {
            let start = self.i;
            while self.i < self.b.len() {
                let c = self.b[self.i];
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            if self.i > start {
                out.push_str(
                    std::str::from_utf8(&self.b[start..self.i])
                        .map_err(|_| CertError::malformed("non-UTF-8 string content"))?,
                );
            }
            let c = *self
                .b
                .get(self.i)
                .ok_or_else(|| CertError::malformed("unterminated string"))?;
            if c == b'"' {
                self.i += 1;
                return Ok(out);
            }
            if c == b'\\' {
                self.i += 1;
                let esc = *self
                    .b
                    .get(self.i)
                    .ok_or_else(|| CertError::malformed("unterminated escape"))?;
                self.i += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if self.i + 4 > self.b.len() {
                            return Err(CertError::malformed("truncated \\u escape"));
                        }
                        let hex = std::str::from_utf8(&self.b[self.i..self.i + 4])
                            .map_err(|_| CertError::malformed("bad \\u escape"))?;
                        let cp = u32::from_str_radix(hex, 16)
                            .map_err(|_| CertError::malformed("bad \\u escape"))?;
                        self.i += 4;
                        let ch = char::from_u32(cp)
                            .ok_or_else(|| CertError::malformed("bad \\u code point"))?;
                        out.push(ch);
                    }
                    _ => return Err(CertError::malformed("unknown escape")),
                }
                continue;
            }
            return Err(CertError::malformed("control byte in string"));
        }
    }

    fn number(&mut self) -> Result<Json, CertError> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
            self.i += 1;
        }
        if self.i < self.b.len() && matches!(self.b[self.i], b'.' | b'e' | b'E') {
            return Err(CertError::malformed("non-integer number in index"));
        }
        let text = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| CertError::malformed("bad number"))?;
        let n: i64 = text
            .parse()
            .map_err(|_| CertError::malformed("bad integer"))?;
        Ok(Json::Num(n))
    }

    fn array(&mut self) -> Result<Json, CertError> {
        self.i += 1; // '['
        let mut items = Vec::new();
        loop {
            if self.peek() == Some(b']') {
                self.i += 1;
                return Ok(Json::Arr(items));
            }
            items.push(self.value()?);
            match self
                .peek()
                .ok_or_else(|| CertError::malformed("unterminated array"))?
            {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(CertError::malformed("expected ',' or ']' in array")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, CertError> {
        self.i += 1; // '{'
        let mut items = Vec::new();
        loop {
            if self.peek() == Some(b'}') {
                self.i += 1;
                return Ok(Json::Obj(items));
            }
            if self.peek() != Some(b'"') {
                return Err(CertError::malformed("object key must be a string"));
            }
            let key = self.string()?;
            if self.peek() != Some(b':') {
                return Err(CertError::malformed("expected ':' in object"));
            }
            self.i += 1;
            let v = self.value()?;
            items.push((key, v));
            match self
                .peek()
                .ok_or_else(|| CertError::malformed("unterminated object"))?
            {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Ok(Json::Obj(items));
                }
                _ => return Err(CertError::malformed("expected ',' or '}' in object")),
            }
        }
    }
}

/// Canonical JSON string escaping (compact; control chars escaped).
fn jstr(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A quoted JSON string (the inverse of reading one).
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    jstr(&mut out, s);
    out
}

// ---------------------------------------------------------------------------
// Canonical index serialization
// ---------------------------------------------------------------------------

impl CertIndex {
    /// The canonical index text WITHOUT the `package_digest` field — the
    /// exact bytes B4 hashes.
    pub fn canonical_pre_digest(&self) -> String {
        let mut out = String::with_capacity(256 + self.modules.len() * 160);
        self.write_header(&mut out);
        self.write_modules(&mut out);
        self.write_certifiers(&mut out);
        self.write_members(&mut out);
        out.push('}');
        out
    }

    /// The full canonical index (with `package_digest`).
    pub fn canonical(&self) -> String {
        let mut out = String::with_capacity(256 + self.modules.len() * 160);
        self.write_header(&mut out);
        self.write_modules(&mut out);
        self.write_certifiers(&mut out);
        self.write_members(&mut out);
        out.push_str(",\"package_digest\":");
        match &self.package_digest {
            Some(d) => jstr(&mut out, d),
            None => out.push_str("null"),
        }
        out.push('}');
        out
    }

    fn write_header(&self, out: &mut String) {
        out.push_str("{\"schema\":");
        jstr(out, &self.schema);
        out.push_str(",\"image\":{\"name\":");
        jstr(out, &self.image_name);
        out.push_str(",\"fingerprint\":");
        jstr(out, &self.image_fingerprint);
        out.push_str("},\"semantics\":");
        jstr(out, &self.semantics);
        out.push_str(",\"stmt\":");
        jstr(out, &self.stmt);
        out.push_str(
            ",\"registries\":{\"statements\":{\"doc\":\"formal-semantics-core.md\",\"rev\":\"1\"},\
             \"certifiers\":{\"doc\":\"verification-trust.md\",\"rev\":\"1\"}},\"policy\":",
        );
        jstr(out, &self.policy);
    }

    fn write_modules(&self, out: &mut String) {
        out.push_str(",\"modules\":[");
        for (i, m) in self.modules.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            out.push_str("{\"name\":");
            jstr(out, &m.name);
            out.push_str(",\"module_digest\":");
            match &m.module_digest {
                Some(d) => jstr(out, d),
                None => out.push_str("null"),
            }
            out.push_str(",\"manifest_digest\":");
            match &m.manifest_digest {
                Some(d) => jstr(out, d),
                None => out.push_str("null"),
            }
            out.push_str(",\"obl_digest\":");
            jstr(out, &m.obl_digest);
            out.push_str(",\"verdicts_digest\":");
            jstr(out, &m.verdicts_digest);
            out.push_str(",\"target\":");
            jstr(out, &m.target);
            out.push_str(",\"model\":");
            jstr(out, &m.model);
            out.push_str(",\"counts\":{\"proof\":");
            out.push_str(&m.counts[0].to_string());
            out.push_str(",\"checked\":");
            out.push_str(&m.counts[1].to_string());
            out.push_str(",\"assumed\":");
            out.push_str(&m.counts[2].to_string());
            out.push_str(",\"open\":");
            out.push_str(&m.counts[3].to_string());
            out.push_str("},\"candidate_ratio\":");
            out.push_str(&m.candidate_ratio.to_string());
            out.push_str(",\"claims\":{\"kinds\":{");
            for (k, (kind, count)) in m.claims.kinds.iter().enumerate() {
                if k != 0 {
                    out.push(',');
                }
                jstr(out, kind);
                out.push(':');
                out.push_str(&count.to_string());
            }
            out.push_str("},\"intents\":[");
            for (k, intent) in m.claims.intents.iter().enumerate() {
                if k != 0 {
                    out.push(',');
                }
                jstr(out, intent);
            }
            out.push_str("]}}");
        }
        out.push(']');
    }

    fn write_certifiers(&self, out: &mut String) {
        out.push_str(",\"certifiers\":[");
        for (i, c) in self.certifiers.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            out.push_str("{\"class\":");
            jstr(out, &c.class);
            out.push_str(",\"name\":");
            jstr(out, &c.name);
            out.push_str(",\"recognition\":");
            jstr(out, &c.recognition);
            out.push_str(",\"tool\":{\"name\":");
            jstr(out, &c.tool_name);
            out.push_str(",\"version\":");
            jstr(out, &c.tool_version);
            out.push_str("},\"toolchain\":");
            jstr(out, &c.toolchain);
            out.push_str(",\"methods\":[");
            for (k, m) in c.methods.iter().enumerate() {
                if k != 0 {
                    out.push(',');
                }
                jstr(out, m);
            }
            out.push_str("]}");
        }
        out.push(']');
    }

    fn write_members(&self, out: &mut String) {
        out.push_str(",\"members\":[");
        for (i, m) in self.members.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            out.push_str("{\"path\":");
            jstr(out, &m.path);
            out.push_str(",\"sha256\":");
            jstr(out, &m.sha256);
            out.push_str(",\"size\":");
            out.push_str(&m.size.to_string());
            out.push('}');
        }
        out.push(']');
    }
}

// ---------------------------------------------------------------------------
// Index reader (the `cert_index_decode` fuzz surface)
// ---------------------------------------------------------------------------

/// Parse a `cert.json` index (fail-closed). Unknown keys are skipped
/// (additive growth); the closed schema version is enforced; fields are
/// validated. This is the reader `tyu cert verify|show|diff` and the E6503
/// deploy-time checks share.
pub fn parse_index(bytes: &[u8]) -> Result<CertIndex, CertError> {
    if bytes.len() > INDEX_MAX_BYTES {
        return Err(CertError::malformed(format!(
            "index exceeds {} bytes",
            INDEX_MAX_BYTES
        )));
    }
    let Json::Obj(pairs) = parse_json(bytes)? else {
        return Err(CertError::malformed("index root must be an object"));
    };

    let mut schema: Option<String> = None;
    let mut image_name: Option<String> = None;
    let mut image_fingerprint: Option<String> = None;
    let mut semantics: Option<String> = None;
    let mut stmt: Option<String> = None;
    let mut policy: Option<String> = None;
    let mut modules: Vec<CertModule> = Vec::new();
    let mut certifiers: Vec<CertificateId> = Vec::new();
    let mut members: Vec<Member> = Vec::new();
    let mut package_digest: Option<String> = None;

    for (key, v) in pairs {
        match key.as_str() {
            "schema" => schema = Some(obj_str(&v, "schema")?),
            "image" => {
                let Json::Obj(im) = v else {
                    return Err(CertError::malformed("image must be an object"));
                };
                for (k, iv) in im {
                    match k.as_str() {
                        "name" => image_name = Some(obj_str(&iv, "image.name")?),
                        "fingerprint" => {
                            image_fingerprint = Some(obj_str(&iv, "image.fingerprint")?)
                        }
                        _ => {}
                    }
                }
            }
            "semantics" => semantics = Some(obj_str(&v, "semantics")?),
            "stmt" => stmt = Some(obj_str(&v, "stmt")?),
            "policy" => policy = Some(obj_str(&v, "policy")?),
            "modules" => modules = parse_modules(&v)?,
            "certifiers" => certifiers = parse_certifiers(&v)?,
            "members" => members = parse_members(&v)?,
            "package_digest" => package_digest = Some(obj_str(&v, "package_digest")?),
            // Unknown keys: skipped (additive growth).
            _ => {}
        }
    }

    let schema = schema.ok_or_else(|| CertError::malformed("missing schema"))?;
    if schema != CERT_SCHEMA {
        return Err(CertError::malformed(format!(
            "unsupported cert schema '{schema}' (expected {CERT_SCHEMA})"
        )));
    }
    let required = "missing index field";
    let index = CertIndex {
        schema,
        image_name: image_name.ok_or_else(|| CertError::malformed(required))?,
        image_fingerprint: image_fingerprint.ok_or_else(|| CertError::malformed(required))?,
        semantics: semantics.ok_or_else(|| CertError::malformed(required))?,
        stmt: stmt.ok_or_else(|| CertError::malformed(required))?,
        policy: policy.ok_or_else(|| CertError::malformed(required))?,
        modules,
        certifiers,
        members,
        package_digest,
    };
    // Canonical-order enforcement (the writer sorts; a hand-reordered index
    // is evidence of tampering).
    let mut prev: Option<&str> = None;
    for m in &index.members {
        if m.path.len() > MEMBER_PATH_MAX {
            return Err(CertError::malformed("member path too long"));
        }
        if prev.is_some_and(|p| p >= m.path.as_str()) {
            return Err(CertError::malformed(
                "members not in canonical (sorted) order",
            ));
        }
        prev = Some(&m.path);
    }
    let mut prev_name: Option<&str> = None;
    for m in &index.modules {
        if prev_name.is_some_and(|p| p >= m.name.as_str()) {
            return Err(CertError::malformed(
                "modules not in canonical (sorted) order",
            ));
        }
        prev_name = Some(&m.name);
    }
    Ok(index)
}

fn obj_str(v: &Json, field: &str) -> Result<String, CertError> {
    match v {
        Json::Str(s) => Ok(s.clone()),
        _ => Err(CertError::malformed(format!("{field} must be a string"))),
    }
}

fn obj_u64_num(v: &Json, field: &str) -> Result<u64, CertError> {
    match v {
        Json::Num(n) if *n >= 0 => Ok(*n as u64),
        _ => Err(CertError::malformed(format!(
            "{field} must be a non-negative integer"
        ))),
    }
}

fn parse_modules(v: &Json) -> Result<Vec<CertModule>, CertError> {
    let Json::Arr(items) = v else {
        return Err(CertError::malformed("modules must be an array"));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Json::Obj(fields) = item else {
            return Err(CertError::malformed("module entry must be an object"));
        };
        let mut name = None;
        let mut module_digest = None;
        let mut manifest_digest = None;
        let mut obl_digest = None;
        let mut verdicts_digest = None;
        let mut target = None;
        let mut model = None;
        let mut counts: Option<[u32; 4]> = None;
        let mut candidate_ratio = None;
        let mut claims = Claims::default();
        for (k, fv) in fields {
            match k.as_str() {
                "name" => name = Some(obj_str(fv, "modules[].name")?),
                "module_digest" => {
                    module_digest = match fv {
                        Json::Null => None,
                        _ => Some(obj_str(fv, "modules[].module_digest")?),
                    }
                }
                "manifest_digest" => {
                    manifest_digest = match fv {
                        Json::Null => None,
                        _ => Some(obj_str(fv, "modules[].manifest_digest")?),
                    }
                }
                "obl_digest" => obl_digest = Some(obj_str(fv, "modules[].obl_digest")?),
                "verdicts_digest" => {
                    verdicts_digest = Some(obj_str(fv, "modules[].verdicts_digest")?)
                }
                "target" => target = Some(obj_str(fv, "modules[].target")?),
                "model" => model = Some(obj_str(fv, "modules[].model")?),
                "counts" => counts = Some(parse_counts(fv)?),
                "candidate_ratio" => {
                    let n = obj_u64_num(fv, "modules[].candidate_ratio")?;
                    if n > u16::MAX as u64 {
                        return Err(CertError::malformed("candidate_ratio out of range"));
                    }
                    candidate_ratio = Some(n as u16);
                }
                "claims" => claims = parse_claims(fv)?,
                _ => {}
            }
        }
        let required = "module entry missing field";
        out.push(CertModule {
            name: name.ok_or_else(|| CertError::malformed(required))?,
            module_digest,
            manifest_digest,
            obl_digest: obl_digest.ok_or_else(|| CertError::malformed(required))?,
            verdicts_digest: verdicts_digest.ok_or_else(|| CertError::malformed(required))?,
            target: target.ok_or_else(|| CertError::malformed(required))?,
            model: model.ok_or_else(|| CertError::malformed(required))?,
            counts: counts.ok_or_else(|| CertError::malformed(required))?,
            candidate_ratio: candidate_ratio.unwrap_or(0),
            claims,
        });
    }
    Ok(out)
}

fn parse_counts(v: &Json) -> Result<[u32; 4], CertError> {
    let Json::Obj(fields) = v else {
        return Err(CertError::malformed("counts must be an object"));
    };
    let mut out = [0u32; 4];
    for (k, fv) in fields {
        let n = obj_u64_num(fv, "counts field")?;
        let n32 = u32::try_from(n).map_err(|_| CertError::malformed("count out of range"))?;
        match k.as_str() {
            "proof" => out[0] = n32,
            "checked" => out[1] = n32,
            "assumed" => out[2] = n32,
            "open" => out[3] = n32,
            _ => {}
        }
    }
    Ok(out)
}

fn parse_claims(v: &Json) -> Result<Claims, CertError> {
    let Json::Obj(fields) = v else {
        return Err(CertError::malformed("claims must be an object"));
    };
    let mut kinds = Vec::new();
    let mut intents = Vec::new();
    for (k, fv) in fields {
        match k.as_str() {
            "kinds" => {
                let Json::Obj(kinds_obj) = fv else {
                    return Err(CertError::malformed("claims.kinds must be an object"));
                };
                for (kind, count) in kinds_obj {
                    let n = obj_u64_num(count, "claims.kinds value")?;
                    let n32 =
                        u32::try_from(n).map_err(|_| CertError::malformed("count out of range"))?;
                    kinds.push((kind.clone(), n32));
                }
                kinds.sort_by(|a, b| a.0.cmp(&b.0));
            }
            "intents" => {
                let Json::Arr(items) = fv else {
                    return Err(CertError::malformed("claims.intents must be an array"));
                };
                for i in items {
                    intents.push(obj_str(i, "claims.intents entry")?);
                }
                intents.sort();
                intents.dedup();
            }
            _ => {}
        }
    }
    Ok(Claims { kinds, intents })
}

fn parse_certifiers(v: &Json) -> Result<Vec<CertificateId>, CertError> {
    let Json::Arr(items) = v else {
        return Err(CertError::malformed("certifiers must be an array"));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Json::Obj(fields) = item else {
            return Err(CertError::malformed("certifier entry must be an object"));
        };
        let mut class = None;
        let mut name = None;
        let mut recognition = None;
        let mut tool_name = None;
        let mut tool_version = None;
        let mut toolchain = None;
        let mut methods = Vec::new();
        for (k, fv) in fields {
            match k.as_str() {
                "class" => class = Some(obj_str(fv, "certifiers[].class")?),
                "name" => name = Some(obj_str(fv, "certifiers[].name")?),
                "recognition" => recognition = Some(obj_str(fv, "certifiers[].recognition")?),
                "toolchain" => toolchain = Some(obj_str(fv, "certifiers[].toolchain")?),
                "tool" => {
                    let Json::Obj(t) = fv else {
                        return Err(CertError::malformed("certifier tool must be an object"));
                    };
                    for (tk, tv) in t {
                        match tk.as_str() {
                            "name" => tool_name = Some(obj_str(tv, "certifier tool.name")?),
                            "version" => {
                                tool_version = Some(obj_str(tv, "certifier tool.version")?)
                            }
                            _ => {}
                        }
                    }
                }
                "methods" => {
                    let Json::Arr(items2) = fv else {
                        return Err(CertError::malformed("certifier methods must be an array"));
                    };
                    for mm in items2 {
                        methods.push(obj_str(mm, "certifier method")?);
                    }
                }
                _ => {}
            }
        }
        let required = "certifier entry missing field";
        out.push(CertificateId {
            class: class.ok_or_else(|| CertError::malformed(required))?,
            name: name.ok_or_else(|| CertError::malformed(required))?,
            recognition: recognition.ok_or_else(|| CertError::malformed(required))?,
            tool_name: tool_name.unwrap_or_default(),
            tool_version: tool_version.unwrap_or_default(),
            toolchain: toolchain.unwrap_or_default(),
            methods,
        });
    }
    Ok(out)
}

fn parse_members(v: &Json) -> Result<Vec<Member>, CertError> {
    let Json::Arr(items) = v else {
        return Err(CertError::malformed("members must be an array"));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Json::Obj(fields) = item else {
            return Err(CertError::malformed("member entry must be an object"));
        };
        let mut path = None;
        let mut sha256 = None;
        let mut size = None;
        for (k, fv) in fields {
            match k.as_str() {
                "path" => path = Some(obj_str(fv, "members[].path")?),
                "sha256" => sha256 = Some(obj_str(fv, "members[].sha256")?),
                "size" => size = Some(obj_u64_num(fv, "members[].size")?),
                _ => {}
            }
        }
        let required = "member entry missing field";
        out.push(Member {
            path: path.ok_or_else(|| CertError::malformed(required))?,
            sha256: sha256.ok_or_else(|| CertError::malformed(required))?,
            size: size.ok_or_else(|| CertError::malformed(required))?,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Hashing + writing helpers
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Atomic write-if-changed (same discipline as vm_summary.rs).
fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// A member being staged: package-relative path + bytes to write.
struct StagedMember {
    rel: String,
    bytes: Vec<u8>,
}

fn stage(rel: impl Into<String>, bytes: Vec<u8>) -> StagedMember {
    StagedMember {
        rel: rel.into(),
        bytes,
    }
}

/// The port's `lean-toolchain` pin (B3 rerun material / `toolchain.lock`).
fn port_toolchain_pin() -> String {
    let root = crate::platform::workspace_root();
    let pin_path = root.join("verification/ports/lean/lean-toolchain");
    fs::read_to_string(&pin_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// The vector-corpus digest for the conformance/differential evidence:
/// SHA-256 over every `crates/verifier/test-vectors/**` file (path + bytes,
/// sorted by relative path) — the committed corpus the port gate executes.
/// `"absent"` when the corpus is not present (out-of-tree deploy).
fn vectors_corpus_digest() -> String {
    let root = crate::platform::workspace_root();
    let dir = root.join("crates/verifier/test-vectors");
    let mut files: Vec<PathBuf> = Vec::new();
    if dir.is_dir() {
        let _ = collect_files(&dir, &mut files);
    }
    if files.is_empty() {
        return "absent".to_string();
    }
    files.sort();
    let mut hasher = Sha256::new();
    for f in files {
        let rel = f
            .strip_prefix(&dir)
            .unwrap_or(&f)
            .to_string_lossy()
            .into_owned();
        let Ok(bytes) = fs::read(&f) else {
            return "absent".to_string();
        };
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(&bytes);
    }
    hex::encode(hasher.finalize())
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

/// The deploy-side assembly input. All paths are absolute.
pub struct AssembleInput<'a> {
    /// The shipped image path (the package directory is `<image>.tyucert`).
    pub image: &'a Path,
    /// The shipped image bytes (what B1 binds against).
    pub image_bytes: &'a [u8],
    /// The packed root-module bytes — the module container carrying the
    /// signed `verify_manifest` (B2's source). Same as `image_bytes` when the
    /// image is the module itself (unsigned static/dynamic deploy).
    pub module_bytes: &'a [u8],
    pub out_dir: &'a Path,
    pub input: &'a Path,
    pub include_dirs: &'a [PathBuf],
    pub sysroot: Option<&'a Path>,
    /// The deploy's verify-policy requirement (governs the index's `policy`).
    pub policy: DeployVerifyPolicy,
    /// When the image's modules are signed, the package MUST be signed with
    /// the same authority (B4); `Some(key)` writes `package.sig`.
    pub sign_key: Option<&'a [u8; 32]>,
}

/// The package directory for an image path (`<image>.tyucert`).
pub fn package_dir(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_os_string();
    name.push(".tyucert");
    PathBuf::from(&name)
}

fn cert_fail(detail: impl Into<String>) -> crate::error::TyuError {
    crate::error::TyuError::Cert {
        code: E_CERT_PAIRING,
        detail: detail.into(),
    }
}

/// Assemble the certification package for a deploy. Returns `Ok(None)` when
/// the build produced no verification state (a legacy `--verify=off` build
/// has nothing to certify — honest absence, not an error).
pub fn assemble_for_deploy(
    args: &AssembleInput<'_>,
) -> Result<Option<PathBuf>, crate::error::TyuError> {
    use crate::error::TyuError;

    // 1. The module set: every graph module with a `tyu.vm/1` summary (the
    //    same set the deploy pairing walks). The root binds B1/B2.
    let graph = crate::graph::resolve_graph(args.input, args.include_dirs, args.sysroot)
        .map_err(|e| TyuError::Graph(format!("cert assembly: {e}")))?;
    let mut with_state: Vec<(String, PathBuf)> = Vec::new();
    for node in &graph {
        let sum_path = crate::vm_summary::vm_summary_path(args.out_dir, &node.name);
        if sum_path.is_file() {
            with_state.push((node.name.clone(), sum_path));
        }
    }
    if with_state.is_empty() {
        return Ok(None);
    }
    with_state.sort_by(|a, b| a.0.cmp(&b.0));
    let root_name = graph.last().map(|n| n.name.as_str());

    // 2. Per-module members + entries.
    let mut members: Vec<StagedMember> = Vec::new();
    let mut index_modules: Vec<CertModule> = Vec::new();
    let mut certifiers: Vec<CertificateId> = Vec::new();
    let mut candidate_ids: Vec<String> = Vec::new();

    let verify_env = crate::proof::VerifyEnvKey::for_build()?;
    for (module, sum_path) in &with_state {
        let text = fs::read_to_string(sum_path).map_err(TyuError::Io)?;
        let spec = lmod_pack::verify::verify_manifest_from_json(&text)
            .map_err(|e| cert_fail(format!("module {module} summary malformed: {e}")))?;

        // Obligation artifact `<out>/<Module>-<16hex>.obl.json` (exactly one).
        let obl = find_obl_artifact(args.out_dir, module)?;
        let (target, model, obl_digest, claims, _statement_doc) = match &obl {
            Some(path) => {
                let bytes = fs::read(path).map_err(TyuError::Io)?;
                let set = verifier::codec::read_obl(&bytes).map_err(|e| {
                    cert_fail(format!(
                        "module {module} obligation artifact invalid (E{}): {e:?}",
                        e.code()
                    ))
                })?;
                members.push(stage(
                    format!("obligations/{module}.obl.json"),
                    bytes.clone(),
                ));
                let doc = statement_document(module, &set);
                members.push(stage(
                    format!("statements/{module}.stmt.json"),
                    doc.clone().into_bytes(),
                ));
                (
                    set.target.clone(),
                    set.model_semantics.clone(),
                    sha256_hex(&bytes),
                    claims_from(&set),
                    doc,
                )
            }
            None => (
                spec.target.clone(),
                spec.model.clone(),
                String::new(),
                Claims::default(),
                String::new(),
            ),
        };

        // Verdicts member: the canonical v2 re-encode (harvest doc → echo).
        let verdicts = module_verdicts(args.out_dir, module, obl.as_deref(), &verify_env)?;
        let verdicts_bytes = verifier::verdict::encode_verdicts(
            "harvest",
            "0.1.0",
            verdicts.certifier.as_ref(),
            &verdicts.target,
            &verdicts.model_semantics,
            &verdicts.records,
            0,
            &verifier::verdict::EmittedChecksData {
                subtype_range: 0,
                contract: 0,
                mmio_bounds: 0,
            },
        )
        .map_err(|e| cert_fail(format!("module {module} verdicts encode: {e:?}")))?;
        let verdicts_digest = sha256_hex(&verdicts_bytes);
        members.push(stage(
            format!("verdicts/{module}.verdicts.json"),
            verdicts_bytes,
        ));

        // Proof material (B3) + candidate ids (\u00A7Q10 attribution).
        stage_proofs(module, &verdicts, args.input, &mut members)?;
        for r in &verdicts.records {
            if r.authored == Some(verifier::verdict::Authored::Candidate) {
                candidate_ids.push(r.id.clone());
            }
        }

        // The recognized certifier who actually certified.
        if let Some(c) = &verdicts.certifier {
            if verifier::verdict::RECOGNIZED_CERTIFIERS.contains(&c.recognition.as_str()) {
                certifiers.push(CertificateId {
                    class: c.class.clone(),
                    name: c.name.clone(),
                    recognition: c.recognition.clone(),
                    tool_name: c.tool.name.clone(),
                    tool_version: c.tool.version.clone(),
                    toolchain: c.toolchain.clone(),
                    methods: methods_used(&verdicts.records),
                });
            }
        }

        // The root binds B1/B2; callee modules carry state but no
        // independently-shipped bytes (they are linked into the root image).
        let (module_digest, manifest_digest) = if Some(module.as_str()) == root_name {
            (
                Some(sha256_hex(args.image_bytes)),
                manifest_digest_of(args.module_bytes),
            )
        } else {
            (None, None)
        };

        index_modules.push(CertModule {
            name: module.clone(),
            module_digest,
            manifest_digest,
            obl_digest,
            verdicts_digest,
            target,
            model,
            counts: spec.counts,
            candidate_ratio: spec.candidate_ratio,
            claims,
        });
    }
    index_modules.sort_by(|a, b| a.name.cmp(&b.name));
    certifiers.sort_by(|a, b| a.recognition.cmp(&b.recognition));
    certifiers.dedup_by(|a, b| a.recognition == b.recognition);

    // 3. Evidence members.
    members.extend(evidence_members(args.out_dir, &with_state, &candidate_ids)?);

    // 4. Index + B4 digest + package.sig.
    let image_name = args
        .image
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string();
    let mut index = CertIndex {
        schema: CERT_SCHEMA.to_string(),
        image_name,
        image_fingerprint: sha256_hex(args.image_bytes),
        semantics: verifier::semantics::SEMANTICS_VERSION.to_string(),
        stmt: verifier::stmt::STMT_SCHEMA.to_string(),
        policy: deploy_policy_str(args.policy).to_string(),
        modules: index_modules,
        certifiers,
        members: Vec::new(),
        package_digest: None,
    };

    // Write the staged members first (content-addressed), then index.
    let pkg_dir = package_dir(args.image);
    for m in &members {
        let path = pkg_dir.join(&m.rel);
        write_if_changed(&path, &m.bytes)
            .map_err(|e| cert_fail(format!("writing package member '{}': {e}", m.rel)))?;
    }
    let mut member_recs: Vec<Member> = members
        .iter()
        .map(|m| Member {
            path: m.rel.clone(),
            sha256: sha256_hex(&m.bytes),
            size: m.bytes.len() as u64,
        })
        .collect();
    member_recs.sort_by(|a, b| a.path.cmp(&b.path));
    index.members = member_recs;

    // B4: package_digest over the canonical index (digest field excluded).
    let digest = sha256_hex(index.canonical_pre_digest().as_bytes());
    index.package_digest = Some(digest.clone());
    write_if_changed(&pkg_dir.join("cert.json"), index.canonical().as_bytes())
        .map_err(|e| cert_fail(format!("writing cert.json: {e}")))?;

    // package.sig — required when the image's modules are signed (B4).
    if let Some(key) = args.sign_key {
        write_package_sig(&pkg_dir, &digest, key)?;
    }

    // FR-21 (E6503): the deploy MUST verify B1/B2 for every module before
    // shipping. Re-run the binder in place (B2 against the packed module
    // bytes — the source the loader validates after decrypt) — fail-closed.
    let checks = verify_package_inner(&pkg_dir, Some(args.image), Some(args.module_bytes), None)?;
    let failures: Vec<String> = checks
        .iter()
        .filter(|c| c.starts_with("FAIL"))
        .cloned()
        .collect();
    if !failures.is_empty() {
        return Err(cert_fail(format!(
            "FR-21 pre-ship binding failed: {}",
            failures.join("; ")
        )));
    }

    Ok(Some(pkg_dir))
}

fn methods_used(records: &[verifier::verdict::VerdictRecord]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in records {
        if let Some(m) = r.method {
            let s = m.as_str().to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out.sort();
    out
}

/// `tyu.cert` policy strings map the deploy requirement onto the manifest
/// policy vocabulary (§6.5/§Q6: the index records the image's declared
/// policy).
fn deploy_policy_str(p: DeployVerifyPolicy) -> &'static str {
    match p {
        DeployVerifyPolicy::OpenOk => "open-ok",
        DeployVerifyPolicy::NoOpen => "no-open",
        DeployVerifyPolicy::Proven => "proven",
    }
}

/// Find the build's obligation artifact for a module: `<out>/<Module>-
/// <16hex>.obl.json`, requiring EXACTLY one (ambiguity fails closed).
fn find_obl_artifact(
    out_dir: &Path,
    module: &str,
) -> Result<Option<PathBuf>, crate::error::TyuError> {
    let mut hits = Vec::new();
    let rd = fs::read_dir(out_dir).map_err(crate::error::TyuError::Io)?;
    for e in rd.flatten() {
        let p = e.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(rest) = name.strip_prefix(&format!("{module}-")) {
            if rest.len() == 20 && rest.ends_with(".obl.json") {
                hits.push(p);
            }
        }
    }
    hits.sort();
    match hits.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        _ => Err(cert_fail(format!(
            "obligation artifact for module {module} is ambiguous ({} candidates in {})",
            hits.len(),
            out_dir.display()
        ))),
    }
}

/// The per-module `tyu.verdicts/v2` document (harvest doc ⇒ echo slot) —
/// same resolution the manifest summaries use.
fn module_verdicts(
    out_dir: &Path,
    module: &str,
    obl_path: Option<&Path>,
    verify_env: &crate::proof::VerifyEnvKey,
) -> Result<verifier::verdict::Verdicts, crate::error::TyuError> {
    if let Some(obl) = obl_path {
        if let Some(v) = crate::vm_summary::verdicts_for_module(out_dir, module, obl, verify_env)? {
            return Ok(v);
        }
    } else {
        let harvest = crate::vm_summary::harvest_verdicts_path(out_dir, module);
        if harvest.is_file() {
            if let Ok(bytes) = fs::read(&harvest) {
                if let Ok(v) = verifier::verdict::read_verdicts(&bytes) {
                    return Ok(v);
                }
            }
        }
    }
    Ok(verifier::verdict::Verdicts {
        semantics: verifier::semantics::SEMANTICS_VERSION.to_string(),
        stmt: verifier::stmt::STMT_SCHEMA.to_string(),
        certifier: None,
        target: String::new(),
        model_semantics: String::new(),
        records: Vec::new(),
    })
}

/// The canonical `statements/<Module>.stmt.json` member (the statement set
/// the P1.3 goldens pin, under the stmt schema).
fn statement_document(module: &str, set: &verifier::model::OblSet) -> String {
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for o in &set.obligations {
        let word_ir_hash = set
            .facts
            .words
            .iter()
            .find(|w| w.name == o.site.word)
            .map(|w| verifier::stmt::sha256_hex16(w.ir.as_bytes()))
            .unwrap_or_default();
        let ctx = verifier::stmt::StatementContext::for_obligation(
            module,
            &set.target,
            &set.model_semantics,
            &word_ir_hash,
            o,
        );
        rows.push((
            o.id.clone(),
            o.id_hash.clone(),
            ctx.statement_hash_hex(&o.formula),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::with_capacity(128 + rows.len() * 160);
    out.push_str("{\"schema\":");
    jstr(&mut out, STATEMENTS_DOC_SCHEMA);
    out.push_str(",\"semantics\":");
    jstr(&mut out, verifier::semantics::SEMANTICS_VERSION);
    out.push_str(",\"stmt\":");
    jstr(&mut out, verifier::stmt::STMT_SCHEMA);
    out.push_str(",\"target\":");
    jstr(&mut out, &set.target);
    out.push_str(",\"model_semantics\":");
    jstr(&mut out, &set.model_semantics);
    out.push_str(",\"module\":");
    jstr(&mut out, module);
    out.push_str(",\"statements\":[");
    for (i, (id, id_hash, stmt_hash)) in rows.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        jstr(&mut out, id);
        out.push_str(",\"id_hash\":");
        jstr(&mut out, id_hash);
        out.push_str(",\"statement_hash\":");
        jstr(&mut out, stmt_hash);
        out.push('}');
    }
    out.push_str("]}\n");
    out
}

/// §Q17 claims from a module's obligation artifact.
fn claims_from(set: &verifier::model::OblSet) -> Claims {
    let mut kind_counts: std::collections::BTreeMap<String, u32> = Default::default();
    let mut intents = Vec::new();
    for o in &set.obligations {
        *kind_counts.entry(o.kind.as_str().to_string()).or_insert(0) += 1;
        if o.intent.authored && !o.intent.label.trim().is_empty() {
            let label = o.intent.label.trim().to_string();
            if !intents.contains(&label) {
                intents.push(label);
            }
        }
    }
    intents.sort();
    Claims {
        kinds: kind_counts.into_iter().collect(),
        intents,
    }
}

/// The B2 source: the root module container's signed `verify_manifest`
/// verdict digest.
fn manifest_digest_of(module_bytes: &[u8]) -> Option<String> {
    let container = lmod::validate::Container::parse(module_bytes).ok()?;
    let vm = lmod::verify_manifest::scan_verify_manifest(container.modinfo())
        .ok()
        .flatten()?;
    Some(hex::encode(vm.digest))
}

/// Stage the proof material for a module (B3): per certificate verdict, the
/// theorem's source file (`proof.file`, resolved against the deploy context)
/// as `proofs/<Module>/<id_hash>/proof.lean`, plus `rerun.json`.
fn stage_proofs(
    module: &str,
    verdicts: &verifier::verdict::Verdicts,
    input: &Path,
    members: &mut Vec<StagedMember>,
) -> Result<(), crate::error::TyuError> {
    let mut seen: Vec<String> = Vec::new(); // dedup by id_hash
    for r in &verdicts.records {
        let Some(p) = &r.proof else { continue };
        if p.kind != verifier::verdict::ProofKind::Certificate {
            continue;
        }
        if seen.contains(&r.id_hash) {
            continue;
        }
        seen.push(r.id_hash.clone());
        let rel_dir = format!("proofs/{module}/{}/", r.id_hash);
        let Some(recorded) = p.file.as_deref() else {
            if r.status == verifier::verdict::VerdictStatus::Discharged {
                return Err(cert_fail(format!(
                    "certificate verdict {} has no proof.file to certify",
                    r.id
                )));
            }
            continue;
        };
        let resolved = resolve_proof_file(input, recorded).ok_or_else(|| {
            cert_fail(format!(
                "certificate verdict {} references '{}' but the theorem file \
                     cannot be resolved from the deploy context",
                r.id, recorded
            ))
        })?;
        let bytes = fs::read(&resolved).map_err(crate::error::TyuError::Io)?;
        members.push(stage(format!("{rel_dir}proof.lean"), bytes));

        let pin = port_toolchain_pin();
        let rerun = format!(
            "{{\"schema\":{},\"method\":\"certificate\",\"port\":\"lean\",\"toolchain\":{},\
             \"commands\":[\"lake build\",\"harvest\"],\"theorem\":{}}}\n",
            json_str(RERUN_SCHEMA),
            json_str(&pin),
            json_str(p.theorem.as_deref().unwrap_or("")),
        );
        members.push(stage(format!("{rel_dir}rerun.json"), rerun.into_bytes()));
    }
    Ok(())
}

/// Resolve a recorded proof-file path (`proofs/<Module>.lean`,
/// `proofs/candidates/<id>.lean`, …) against the deploy context: CWD, the
/// input's directory, then every ancestor of the input.
fn resolve_proof_file(input: &Path, recorded: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(PathBuf::from(recorded));
    if let Some(dir) = input.parent() {
        candidates.push(dir.join(recorded));
        let mut ancestor = dir;
        loop {
            match ancestor.parent() {
                Some(p) if p != ancestor => {
                    candidates.push(p.join(recorded));
                    ancestor = p;
                }
                _ => break,
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The `evidence/` members: per-module axiom audits, candidate ids, the
/// per-bundle toolchain lock, the §6.8 TCB boundary, and the
/// conformance/differential corpus digests (honest "collected" state — the
/// port gate is the blocking executor).
fn evidence_members(
    out_dir: &Path,
    modules_with_state: &[(String, PathBuf)],
    candidate_ids: &[String],
) -> Result<Vec<StagedMember>, crate::error::TyuError> {
    let mut members: Vec<StagedMember> = Vec::new();
    let harvest_dir = crate::vm_summary::verify_state_dir(out_dir).join("harvest");

    // axiom_audit.json — assemble the per-module `tyu.axiom-audit/1` docs.
    let mut audits: Vec<String> = Vec::new();
    for (module, _) in modules_with_state {
        let p = harvest_dir.join(format!("{module}.axiom_audit.json"));
        if p.is_file() {
            if let Ok(text) = fs::read_to_string(&p) {
                audits.push(format!(
                    "{{\"module\":{},\"audit\":{}}}",
                    json_str(module),
                    text.trim()
                ));
            }
        }
    }
    members.push(stage(
        "evidence/axiom_audit.json",
        format!(
            "{{\"schema\":{},\"modules\":[{}]}}\n",
            json_str(AXIOM_AUDIT_SCHEMA),
            audits.join(",")
        )
        .into_bytes(),
    ));

    // candidates.json — the candidate-authored certificate ids (§Q10).
    let mut cands: Vec<String> = candidate_ids.to_vec();
    cands.sort();
    cands.dedup();
    members.push(stage(
        "evidence/candidates.json",
        format!(
            "{{\"schema\":{},\"candidates\":[{}]}}\n",
            json_str(CANDIDATES_SCHEMA),
            cands
                .iter()
                .map(|i| json_str(i))
                .collect::<Vec<_>>()
                .join(",")
        )
        .into_bytes(),
    ));

    // toolchain.lock — the port's pinned toolchain identity.
    let pin = port_toolchain_pin();
    members.push(stage(
        "evidence/toolchain.lock",
        format!("schema-contract lean4 pin {pin}\n").into_bytes(),
    ));

    // tcb.json — the §6.8 boundary as shipped (the report's TCB).
    let tcb = verifier::report::VerifyReport::default_tcb();
    let mut out = format!("{{\"schema\":{},\"tcb\":[", json_str(TCB_SCHEMA));
    for (i, t) in tcb.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"id\":{},\"what\":{},\"status\":{}}}",
            json_str(&t.id),
            json_str(&t.what),
            json_str(&t.status),
        ));
    }
    out.push_str("]}\n");
    members.push(stage("evidence/tcb.json", out.into_bytes()));

    // conformance.json + differential.json — corpus digests + the honest
    // collected state (the port gate is the executer).
    let corpus = vectors_corpus_digest();
    members.push(stage(
        "evidence/conformance.json",
        format!(
            "{{\"schema\":{},\"corpus_digest\":{},\"port\":\"lean\",\"status\":\"collected\",\
             \"executor\":\"ci/port.sh P3.2 conformance\"}}\n",
            json_str(CONFORMANCE_SCHEMA),
            json_str(&corpus),
        )
        .into_bytes(),
    ));
    members.push(stage(
        "evidence/differential.json",
        format!(
            "{{\"schema\":{},\"corpus_digest\":{},\"programs\":0,\"status\":\"collected\",\
             \"executor\":\"ci/port.sh P14 differential (post-T-A/T-B)\"}}\n",
            json_str(DIFFERENTIAL_SCHEMA),
            json_str(&corpus),
        )
        .into_bytes(),
    ));

    Ok(members)
}

/// Write `package.sig` — detached HMAC-SHA256 over the package digest (B4),
/// the same key surface as `lmod-sign`.
fn write_package_sig(
    pkg_dir: &Path,
    digest: &str,
    key: &[u8; 32],
) -> Result<(), crate::error::TyuError> {
    let tag = lmod_sign::hmac_sha256(key, digest.as_bytes());
    let mut out = Vec::with_capacity(1 + tag.len());
    out.push(lmod::sig::SCHEME_HMAC_SHA256);
    out.extend_from_slice(&tag);
    write_if_changed(&pkg_dir.join("package.sig"), &out)
        .map_err(|e| cert_fail(format!("writing package.sig: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Verification (B4 → B1 → B2 → B3), fail-fast
// ---------------------------------------------------------------------------

/// The default image path for a package: `<pkg>` minus the `.tyucert` suffix.
pub fn default_image_path(pkg: &Path) -> PathBuf {
    let s = pkg.to_string_lossy();
    if let Some(stripped) = s.strip_suffix(".tyucert") {
        PathBuf::from(stripped)
    } else {
        pkg.to_path_buf()
    }
}

/// Verify a certification package: B4 → B1 → B2 → B3, fail-fast. The image
/// defaults to the sibling of the package (`<image>.tyucert` → `<image>`);
/// pass `--image` to bind an explicit path. Returns the human-readable check
/// lines; any failure is a hard `Err`.
pub fn verify_package(
    pkg: &Path,
    image: Option<&Path>,
) -> Result<Vec<String>, crate::error::TyuError> {
    verify_package_inner(pkg, image, None, None)
}

/// `verify_package` plus `package.sig` HMAC verification (auditor with the
/// signing key).
pub fn verify_package_signed(
    pkg: &Path,
    image: Option<&Path>,
    sign_key: Option<&[u8; 32]>,
) -> Result<Vec<String>, crate::error::TyuError> {
    verify_package_inner(pkg, image, None, sign_key)
}

/// The shared binder. `b2_bytes` overrides the source the B2 digest is read
/// from (the deploy passes the packed pre-encryption module bytes — the
/// source the loader validates after decrypt; an auditor omits it and B2
/// reads the shipped image).
fn verify_package_inner(
    pkg: &Path,
    image: Option<&Path>,
    b2_bytes: Option<&[u8]>,
    sign_key: Option<&[u8; 32]>,
) -> Result<Vec<String>, crate::error::TyuError> {
    use crate::error::TyuError;

    let cert_path = pkg.join("cert.json");
    if !cert_path.is_file() {
        return Err(TyuError::VerifyPairing {
            code: E_CERT_PAIRING,
            detail: format!("no cert.json in {}", pkg.display()),
        });
    }
    let index_bytes = fs::read(&cert_path).map_err(TyuError::Io)?;
    let index = parse_index(&index_bytes).map_err(TyuError::from)?;
    let mut lines: Vec<String> = Vec::new();

    // ---- B4: package integrity (checked FIRST) ----
    let digest = index
        .package_digest
        .clone()
        .ok_or_else(|| TyuError::VerifyPairing {
            code: E_CERT_PAIRING,
            detail: "index carries no package_digest".into(),
        })?;
    let recomputed = sha256_hex(index.canonical_pre_digest().as_bytes());
    if recomputed != digest {
        return Err(TyuError::VerifyPairing {
            code: E_CERT_PAIRING,
            detail: format!(
                "B4: package_digest mismatch — index tampered (stored {digest}, \
                 recomputed {recomputed})"
            ),
        });
    }
    lines.push("PASS B4: package_digest recomputes over the canonical index".into());
    if let Some(key) = sign_key {
        let sig_bytes = fs::read(pkg.join("package.sig")).map_err(|_| TyuError::VerifyPairing {
            code: E_CERT_PAIRING,
            detail: "B4: package.sig missing but a signing key was supplied".into(),
        })?;
        if sig_bytes.first() != Some(&lmod::sig::SCHEME_HMAC_SHA256) || sig_bytes.len() != 33 {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: "B4: malformed package.sig trailer".into(),
            });
        }
        let expected = lmod_sign::hmac_sha256(key, digest.as_bytes());
        if sig_bytes[1..] != expected {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: "B4: package.sig HMAC does not verify".into(),
            });
        }
        lines.push("PASS B4: package.sig HMAC-SHA256 verifies".into());
    }

    // ---- B1: package → image (module digests) ----
    let image_bytes = match image {
        Some(p) => Some(fs::read(p).map_err(TyuError::Io)?),
        None => {
            let def = default_image_path(pkg);
            if def.is_file() {
                fs::read(&def).map_err(TyuError::Io).ok()
            } else {
                None
            }
        }
    };
    let mut bound_modules = 0;
    for m in &index.modules {
        let Some(expected) = &m.module_digest else {
            continue; // callee — no independently-shipped bytes
        };
        let Some(bytes) = &image_bytes else {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!(
                    "B1: module '{}' declares a module_digest but no image was provided \
                     (pass --image <path>)",
                    m.name
                ),
            });
        };
        if sha256_hex(bytes) != *expected {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!(
                    "B1: module '{}' digest mismatch — the image is not the certified \
                     artifact (E6503)",
                    m.name
                ),
            });
        }
        bound_modules += 1;
    }
    if bound_modules == 0 && !index.modules.is_empty() {
        return Err(TyuError::VerifyPairing {
            code: E_CERT_PAIRING,
            detail: "B1: the index declares no shipped-module digest to bind".into(),
        });
    }
    lines.push(format!(
        "PASS B1: {bound_modules} shipped module(s) bind to the image digest"
    ));

    // ---- B2: package → signed module (manifest digests) ----
    let b2_source: Vec<u8> = match b2_bytes {
        Some(b) => b.to_vec(),
        None => image_bytes.clone().unwrap_or_default(),
    };
    let mut bound_manifests = 0;
    for m in &index.modules {
        let Some(expected) = &m.manifest_digest else {
            continue;
        };
        if b2_source.is_empty() {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!(
                    "B2: module '{}' declares a manifest_digest but no image/module \
                     bytes were provided",
                    m.name
                ),
            });
        }
        let container =
            lmod::validate::Container::parse(&b2_source).map_err(|_| TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: "B2: the image is not a valid .lmod container".into(),
            })?;
        let vm = lmod::verify_manifest::scan_verify_manifest(container.modinfo())
            .map_err(|_| TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: "B2: the image's verify_manifest is malformed".into(),
            })?
            .ok_or_else(|| TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!(
                    "B2: module '{}' declares a manifest_digest but the shipped \
                         module bytes carry no verify_manifest record",
                    m.name
                ),
            })?;
        if hex::encode(vm.digest) != *expected {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!(
                    "B2: module '{}' manifest verdict_digest mismatch (E6503)",
                    m.name
                ),
            });
        }
        let region = &container.modinfo()[vm.obligations_start..vm.obligations_end];
        let recomputed_manifest = sha256_hex(region);
        if recomputed_manifest.as_bytes() != expected.as_bytes() {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!(
                    "B2: module '{}' manifest digest does not recompute (record tampered)",
                    m.name
                ),
            });
        }
        bound_manifests += 1;
    }
    lines.push(format!(
        "PASS B2: {bound_manifests} module manifest digest(s) bind"
    ));

    // ---- B3: package → members + proof material ----
    let mut checked_members = 0;
    for m in &index.members {
        let p = pkg.join(&m.path);
        let Ok(bytes) = fs::read(&p) else {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!("B3: member '{}' is missing from the package", m.path),
            });
        };
        if bytes.len() as u64 != m.size || sha256_hex(&bytes) != m.sha256 {
            return Err(TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!(
                    "B3: member '{}' digest/size mismatch — content not content-addressed",
                    m.path
                ),
            });
        }
        checked_members += 1;
    }
    lines.push(format!(
        "PASS B3: {checked_members} members content-addressed"
    ));

    for m in &index.modules {
        let vdoc = pkg.join(format!("verdicts/{}.verdicts.json", m.name));
        if !vdoc.is_file() {
            continue;
        }
        let bytes = fs::read(&vdoc).map_err(TyuError::Io)?;
        let verdicts =
            verifier::verdict::read_verdicts(&bytes).map_err(|_| TyuError::VerifyPairing {
                code: E_CERT_PAIRING,
                detail: format!("B3: module '{}' verdicts member unreadable", m.name),
            })?;
        for r in &verdicts.records {
            let Some(p) = &r.proof else { continue };
            if p.kind == verifier::verdict::ProofKind::Certificate
                && r.status == verifier::verdict::VerdictStatus::Discharged
            {
                let proof_lean = pkg.join(format!("proofs/{}/{}/proof.lean", m.name, r.id_hash));
                if !proof_lean.is_file() {
                    return Err(TyuError::VerifyPairing {
                        code: E_CERT_PAIRING,
                        detail: format!(
                            "B3: certificate verdict '{}' has no content-addressed \
                             proof.lean (proofs/{}/{}/)",
                            r.id, m.name, r.id_hash
                        ),
                    });
                }
            }
        }
    }
    lines.push("PASS B3: certificate proof material present per verdict".into());

    Ok(lines)
}

// ---------------------------------------------------------------------------
// show / diff
// ---------------------------------------------------------------------------

/// Render a certification package for humans (`tyu cert show`): per-module
/// trust counts + claims + candidate ratios, the certifier identity, the TCB
/// boundary (§6.8), and the packaging facts.
pub fn show(pkg: &Path) -> Result<String, crate::error::TyuError> {
    use crate::error::TyuError;
    let index_bytes = fs::read(pkg.join("cert.json")).map_err(TyuError::Io)?;
    let index = parse_index(&index_bytes).map_err(TyuError::from)?;
    let mut out = String::new();
    out.push_str(&format!("certificate package: {}\n", pkg.display()));
    out.push_str(&format!(
        "schema {}\nimage {} (fingerprint {})\nsemantics {} stmt {} policy {}\n",
        index.schema,
        index.image_name,
        truncate(&index.image_fingerprint, 16),
        index.semantics,
        index.stmt,
        index.policy
    ));
    if let Some(d) = &index.package_digest {
        out.push_str(&format!(
            "package_digest {} ({} bytes)\n",
            truncate(d, 16),
            index_bytes.len()
        ));
    }
    out.push_str("certifiers:\n");
    if index.certifiers.is_empty() {
        out.push_str("  (none — no recognized producer)\n");
    }
    for c in &index.certifiers {
        out.push_str(&format!(
            "  {} {} [{}] toolchain {} methods [{}]\n",
            c.class,
            c.name,
            c.recognition,
            truncate(&c.toolchain, 40),
            c.methods.join(", ")
        ));
    }
    out.push_str("modules:\n");
    for m in &index.modules {
        out.push_str(&format!(
            "  {}: proof {} checked {} assumed {} open {} candidate_ratio {}\n",
            m.name, m.counts[0], m.counts[1], m.counts[2], m.counts[3], m.candidate_ratio
        ));
        if !m.claims.kinds.is_empty() {
            let kinds = m
                .claims
                .kinds
                .iter()
                .map(|(k, n)| format!("{k}={n}"))
                .collect::<Vec<_>>()
                .join(" ");
            out.push_str(&format!("     kinds: {kinds}\n"));
        }
        if !m.claims.intents.is_empty() {
            out.push_str("     intents:\n");
            for i in &m.claims.intents {
                out.push_str(&format!("       - {i}\n"));
            }
        }
        if let Some(d) = &m.module_digest {
            out.push_str(&format!("     module_digest {}\n", truncate(d, 16)));
        }
        if let Some(d) = &m.manifest_digest {
            out.push_str(&format!("     manifest_digest {}\n", truncate(d, 16)));
        }
    }
    let tcb_path = pkg.join("evidence/tcb.json");
    if tcb_path.is_file() {
        if let Ok(text) = fs::read_to_string(&tcb_path) {
            if let Ok(tcb) = parse_tcb(&text) {
                out.push_str("tcb (shipped boundary):\n");
                for (id, what, status) in tcb {
                    out.push_str(&format!("  {id}: {status} — {what}\n"));
                }
            }
        }
    }
    out.push_str(&format!(
        "members: {} content-addressed files\n",
        index.members.len()
    ));
    Ok(out)
}

/// Parse the `evidence/tcb.json` doc: `[(id, what, status)]`.
fn parse_tcb(text: &str) -> Result<Vec<(String, String, String)>, CertError> {
    let Json::Obj(fields) = parse_json(text.as_bytes())? else {
        return Err(CertError::malformed("tcb must be an object"));
    };
    let mut out = Vec::new();
    for (k, v) in fields {
        if k != "tcb" {
            continue;
        }
        let Json::Arr(items) = v else {
            return Err(CertError::malformed("tcb must be an array"));
        };
        for item in items {
            let Json::Obj(f) = item else {
                continue;
            };
            let mut id = String::new();
            let mut what = String::new();
            let mut status = String::new();
            for (fk, fv) in f {
                match fk.as_str() {
                    "id" => id = obj_str(&fv, "tcb[].id")?,
                    "what" => what = obj_str(&fv, "tcb[].what")?,
                    "status" => status = obj_str(&fv, "tcb[].status")?,
                    _ => {}
                }
            }
            out.push((id, what, status));
        }
    }
    Ok(out)
}

/// Compare two packages structurally (`tyu cert diff`). Prints `-`/`+`/`*`
/// lines; identical packages print one summary line.
pub fn diff(a: &Path, b: &Path) -> Result<String, crate::error::TyuError> {
    use crate::error::TyuError;
    fn load(p: &Path) -> Result<CertIndex, crate::error::TyuError> {
        let bytes = fs::read(p.join("cert.json")).map_err(TyuError::Io)?;
        parse_index(&bytes).map_err(TyuError::from)
    }
    let ia = load(a)?;
    let ib = load(b)?;
    let mut out = String::new();
    let mut differences = 0usize;

    if ia.image_name != ib.image_name || ia.image_fingerprint != ib.image_fingerprint {
        differences += 1;
        out.push_str(&format!(
            "- image {} ({})\n+ image {} ({})\n",
            ia.image_name,
            truncate(&ia.image_fingerprint, 16),
            ib.image_name,
            truncate(&ib.image_fingerprint, 16)
        ));
    }
    if ia.policy != ib.policy || ia.semantics != ib.semantics || ia.stmt != ib.stmt {
        differences += 1;
        out.push_str(&format!(
            "- policy {} semantics {} stmt {}\n+ policy {} semantics {} stmt {}\n",
            ia.policy, ia.semantics, ia.stmt, ib.policy, ib.semantics, ib.stmt
        ));
    }

    let mut mods_a: std::collections::BTreeMap<&str, &CertModule> = Default::default();
    for m in &ia.modules {
        mods_a.insert(m.name.as_str(), m);
    }
    let mut mods_b: std::collections::BTreeMap<&str, &CertModule> = Default::default();
    for m in &ib.modules {
        mods_b.insert(m.name.as_str(), m);
    }
    for (name, mb) in &mods_b {
        match mods_a.get(name) {
            None => {
                differences += 1;
                out.push_str(&format!("+ module {name}\n"));
            }
            Some(ma) => {
                if **ma != **mb {
                    differences += 1;
                    out.push_str(&format!(
                        "* module {name} changed (digests/counts/claims)\n"
                    ));
                }
            }
        }
    }
    for name in mods_a.keys() {
        if !mods_b.contains_key(name) {
            differences += 1;
            out.push_str(&format!("- module {name}\n"));
        }
    }

    let members_a: std::collections::BTreeSet<&str> =
        ia.members.iter().map(|m| m.path.as_str()).collect();
    let members_b: std::collections::BTreeSet<&str> =
        ib.members.iter().map(|m| m.path.as_str()).collect();
    for p in members_b.difference(&members_a) {
        differences += 1;
        out.push_str(&format!("+ member {p}\n"));
    }
    for p in members_a.difference(&members_b) {
        differences += 1;
        out.push_str(&format!("- member {p}\n"));
    }
    for p in members_a.intersection(&members_b) {
        let ma = ia.members.iter().find(|m| m.path == *p).unwrap();
        let mb = ib.members.iter().find(|m| m.path == *p).unwrap();
        if ma.sha256 != mb.sha256 || ma.size != mb.size {
            differences += 1;
            out.push_str(&format!("* member {p} content changed\n"));
        }
    }

    let recs_a: Vec<&str> = ia
        .certifiers
        .iter()
        .map(|c| c.recognition.as_str())
        .collect();
    let recs_b: Vec<&str> = ib
        .certifiers
        .iter()
        .map(|c| c.recognition.as_str())
        .collect();
    if recs_a != recs_b {
        differences += 1;
        out.push_str(&format!(
            "- certifiers [{}]\n+ certifiers [{}]\n",
            recs_a.join(", "),
            recs_b.join(", ")
        ));
    }

    if differences == 0 {
        out.push_str("identical certification packages (structure + members)\n");
    }
    Ok(out)
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    // Never split a UTF-8 character.
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

// ---------------------------------------------------------------------------
// CLI entry (`tyu cert`)
// ---------------------------------------------------------------------------

/// Run a `tyu cert` subcommand (P11.3, FR-9).
pub fn run(args: &crate::args::CertArgs) -> Result<(), crate::error::TyuError> {
    use crate::args::CertArgs;
    match args {
        CertArgs::Verify {
            pkg,
            image,
            key_sign,
        } => {
            let key = match key_sign {
                Some(kr) => {
                    let kr = crate::keys::KeyRef::parse(kr)?;
                    let material = crate::keys::KeyMaterial::resolve(&kr)?;
                    Some(material.try_as_32bytes()?.to_owned())
                }
                None => None,
            };
            let lines = verify_package_signed(pkg, image.as_deref(), key.as_ref())?;
            for line in &lines {
                println!("{line}");
            }
            Ok(())
        }
        CertArgs::Show { pkg } => {
            let text = show(pkg)?;
            print!("{text}");
            Ok(())
        }
        CertArgs::Diff { a, b } => {
            let text = diff(a, b)?;
            print!("{text}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_roundtrip(index: &CertIndex) {
        let full = index.canonical();
        let parsed = parse_index(full.as_bytes()).expect("roundtrip parse");
        assert_eq!(&parsed, index, "canonical index must round-trip");
        let pre = index.canonical_pre_digest();
        let partial = parse_index(pre.as_bytes()).expect("pre-digest parse");
        assert_eq!(partial.package_digest, None);
    }

    fn sample_index() -> CertIndex {
        CertIndex {
            schema: CERT_SCHEMA.to_string(),
            image_name: "firmware.img".to_string(),
            image_fingerprint: "ab".repeat(32),
            semantics: "tyu.ir-sem/1.0".to_string(),
            stmt: "tyu.stmt/1.0".to_string(),
            policy: "proven".to_string(),
            modules: vec![CertModule {
                name: "Bank".to_string(),
                module_digest: Some("cd".repeat(32)),
                manifest_digest: Some("ef".repeat(32)),
                obl_digest: "01".repeat(32),
                verdicts_digest: "23".repeat(32),
                target: "x86_64-unknown-none".to_string(),
                model: "tyu.model/x86_64-unknown-none/1".to_string(),
                counts: [3, 1, 0, 0],
                candidate_ratio: 0,
                claims: Claims {
                    kinds: vec![("contract-post".to_string(), 2u32)],
                    intents: vec!["withdraw never exceeds balance".to_string()],
                },
            }],
            certifiers: vec![CertificateId {
                class: "port".to_string(),
                name: "lean".to_string(),
                recognition: "tyu-port/lean/1".to_string(),
                tool_name: "harvest".to_string(),
                tool_version: "0.1.0".to_string(),
                toolchain: "lean4:4.15.0".to_string(),
                methods: vec!["certificate".to_string()],
            }],
            members: vec![Member {
                path: "verdicts/Bank.verdicts.json".to_string(),
                sha256: "45".repeat(32),
                size: 42,
            }],
            package_digest: Some("67".repeat(32)),
        }
    }

    #[test]
    fn canonical_index_roundtrips() {
        index_roundtrip(&sample_index());
    }

    #[test]
    fn unknown_keys_are_tolerated() {
        let index = sample_index();
        let full = index.canonical();
        // Inject an unknown top-level key before the final `}`.
        let with_extra = {
            let mut s = String::new();
            let mut seen_close = false;
            for ch in full.chars() {
                if ch == '}' && !seen_close {
                    s.push_str(",\"future\":123");
                    seen_close = true;
                }
                s.push(ch);
            }
            s
        };
        let parsed = parse_index(with_extra.as_bytes()).expect("extra key tolerated");
        assert_eq!(parsed.modules.len(), 1);
        assert_eq!(parsed.members.len(), 1);
        assert_eq!(parsed.package_digest, index.package_digest);
    }

    #[test]
    fn closed_schema_is_rejected() {
        let mut index = sample_index();
        index.schema = "tyu.cert/v2".to_string();
        let err = parse_index(index.canonical().as_bytes()).unwrap_err();
        assert_eq!(err.code(), E_CERT_INDEX_MALFORMED);
    }

    #[test]
    fn malformed_inputs_fail_closed() {
        for input in [
            b"".as_slice(),
            b"not json",
            b"{",
            b"[]",
            b"{}",
            b"{\"schema\":\"tyu.cert/v1\"}",
            b"{\"schema\":\"tyu.cert/v1\",\"modules\":[{}]}",
            b"{\"schema\":\"tyu.cert/v1\",\"members\":[{\"path\":\"x\",\"sha256\":\"y\"}]}",
        ] {
            assert!(parse_index(input).is_err(), "input {:?} must fail", input);
        }
    }

    #[test]
    fn deep_nesting_is_depth_capped_not_stack_overflow() {
        let depth: usize = 200;
        let mut input = "[".repeat(depth) + "0" + &"]".repeat(depth);
        if depth < 100 {
            input.push('i');
        }
        let bytes = input.as_bytes();
        // Must not abort: the parser returns an error (not a crash).
        let _ = parse_index(bytes);
        assert!(parse_index(b"[[[[[[[[[[[[[[[[[[[[").is_err());
    }

    #[test]
    fn digest_excluded_form_is_the_hash_input() {
        let index = sample_index();
        let pre = index.canonical_pre_digest();
        let computed = sha256_hex(pre.as_bytes());
        // A real package binds package_digest = sha256(pre-digest form).
        let mut bound = index.clone();
        bound.package_digest = Some(computed.clone());
        let full = bound.canonical();
        assert_ne!(pre, full);
        let reparsed = parse_index(full.as_bytes()).expect("reparse");
        assert_eq!(reparsed.package_digest.as_deref(), Some(computed.as_str()));
    }

    #[test]
    fn json_string_escapes_are_symmetric() {
        let tricky = "quote\" back\\ newline\n tab\t control:\u{0001}";
        let quoted = json_str(tricky);
        let parsed = match parse_json(quoted.as_bytes()).unwrap() {
            Json::Str(s) => s,
            other => panic!("expected string, got {other:?}"),
        };
        assert_eq!(parsed, tricky);
    }
}
