//! Verdicts file codec (static-verification.md §6.2, slice P4).
//!
//! A *verdicts file* is the interface an external tool (or a cached build)
//! uses to tell the compiler which obligations are discharged and which are
//! assumed. It is **untrusted input** (§7.5): schema-validated, size-capped,
//! and fail-closed — a malformed file is a hard error (E6402) that aborts
//! before codegen, never a silent best-effort read.
//!
//! `langc` consumes a verdicts file under `--checks=undischarged` (which the
//! args layer requires to be present, else E6402). Every record carries the
//! obligation's canonical `id` and `id_hash` (Q3): an id whose hash disagrees
//! with the compiler's computation is ignored and counted stale — the lookup
//! can only ever *fail closed* (treat the site as open, keep the check).
//!
//! The same schema doubles as the *echo* langc writes beside the artifact:
//! `<Module>.verdicts.inTree.json` carries the resolved (non-open) records,
//! the module's `stale_verdicts` count, and the `emitted` check accounting
//! that feeds the report's `emitted_checks` field (FR-15). An echo is itself
//! a valid verdicts input, which is what makes tyu's `.tyu-verify` cache
//! round-trip (a cached echo fed back as `--verdicts` reproduces the same
//! decisions — Q11/FR-17).
//!
//! Hand-rolled JSON — deliberately no `serde` dependency (same
//! `-nodefaultlibs` link constraint as [`crate::codec`]). Fixed key order on
//! the write side (Q11); unknown keys are skipped on the read side (additive
//! schema growth), except `status`, whose value set is closed.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::model::Obligation;
use crate::semantics::SEMANTICS_VERSION;
use crate::codec::{push_str_json, push_i64_json};

/// Schema identifier of verdicts files (`tyu.verdicts/v1`).
pub const VERDICTS_SCHEMA: &str = "tyu.verdicts/v1";

/// Hard read-side cap for verdicts files (static-verification.md §7.5:
/// `--verdicts` ≤ 4 MiB; over-cap → E6402, fail-closed).
pub const VERDICTS_FILE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Maximum length of an `id`/`id_hash` string (§7.5 — ids are capped so a
/// hostile file cannot turn the verdicts map into a memory sink).
pub const VERDICT_ID_MAX_BYTES: usize = 512;

/// A verdict's status. The *input* surface is closed to `discharged` and
/// `assumed` (§6.2: anything else is malformed, E6402); `Open` is the default
/// resolution (no record ⇒ open ⇒ check retained) and appears only in the
/// in-tree resolution records, never in an input file.
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

    /// Strict input parse: only `discharged`/`assumed` are legal in a verdicts
    /// file — anything else is malformed (E6402).
    #[allow(clippy::should_implement_trait)]
    pub fn from_file_str(s: &str) -> Option<VerdictStatus> {
        match s {
            "discharged" => Some(VerdictStatus::Discharged),
            "assumed" => Some(VerdictStatus::Assumed),
            _ => None,
        }
    }

    /// Echo parse: also admits `open` (the in-tree resolution echo).
    pub fn from_any_str(s: &str) -> Option<VerdictStatus> {
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

/// One verdict record (§6.2). Field order is the schema order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerdictRecord {
    pub id: String,
    pub id_hash: String,
    pub status: VerdictStatus,
    /// Discharged: the discharge method (`"interval"`, `"descriptor"`, …).
    pub method: Option<String>,
    /// Discharged: optional pointer to a machine-checkable proof artifact.
    pub proof_ref: Option<String>,
    /// Assumed: required human justification.
    pub justification: Option<String>,
}

impl VerdictRecord {
    pub fn discharged(id: String, id_hash: String, method: &str) -> Self {
        Self {
            id,
            id_hash,
            status: VerdictStatus::Discharged,
            method: Some(method.to_string()),
            proof_ref: None,
            justification: None,
        }
    }
}

/// The per-module `emitted` accounting an echo carries (FR-15): how many
/// runtime checks of each class actually made it into the object. This is the
/// honesty field — the report's `emitted_checks` MUST agree with the object
/// code (FR-16 bijection).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct EmittedChecksData {
    /// Emitted subtype-range checks (C1/C2/C3) — equals the open subtype-site
    /// count: every undischarged/unassumed site emits.
    pub subtype_range: u32,
    /// Emitted contract checks (C5/C6 needs/ensures traps). In P4 contracts
    /// are not yet per-site verdict-driven (P6), so this is the emitted-trap
    /// count under every mode.
    pub contract: u32,
    /// Emitted emulated-aperture MMIO bounds checks (C7). Per-word granularity
    /// (Q8): a word with any open access retains all its checks.
    pub mmio_bounds: u32,
}

/// The parsed and schema-validated verdicts document (§6.2). The lookup only
/// ever fails closed: an absent or hash-mismatched record means open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verdicts {
    pub semantics: String,
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
}

/// Verdict-file validation failures (fail-closed E6402, §7.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerdictError {
    /// Not parseable as the artifact shape at all.
    Malformed,
    /// Top-level `schema` is not `tyu.verdicts/v1`.
    SchemaVersion { found: String },
    /// Top-level `semantics` disagrees with `SEMANTICS_VERSION` — caches and
    /// proofs are keyed on it (Q3).
    SemanticsMismatch { found: String },
    /// Exceeds the 4 MiB read-side cap.
    TooLarge { size: usize },
}

impl VerdictError {
    /// The diagnostic code these failures surface as (E6402, fail-closed).
    pub fn code(&self) -> u32 {
        match self {
            VerdictError::SchemaVersion { .. }
            | VerdictError::SemanticsMismatch { .. }
            | VerdictError::Malformed
            | VerdictError::TooLarge { .. } => 6402,
        }
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Serialize a verdicts/echo document. `records` are emitted in order; missing
/// top-level optional keys (`tool` omitted when `tool_name` is empty,
/// `stale_verdicts`/`emitted` omitted when zero) are deterministic —
/// the reader fills defaults.
pub fn encode_verdicts(
    tool_name: &str,
    tool_version: &str,
    records: &[VerdictRecord],
    stale_verdicts: u32,
    emitted: &EmittedChecksData,
) -> Result<Vec<u8>, VerdictError> {
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(b"{\"schema\":");
    push_str_json(&mut out, VERDICTS_SCHEMA);
    out.extend_from_slice(b",\"tool\":{\"name\":");
    push_str_json(&mut out, tool_name);
    out.extend_from_slice(b",\"version\":");
    push_str_json(&mut out, tool_version);
    out.extend_from_slice(b"},\"semantics\":");
    push_str_json(&mut out, SEMANTICS_VERSION);
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
    if let Some(m) = &r.method {
        out.extend_from_slice(b",\"method\":");
        push_str_json(out, m);
    }
    if let Some(p) = &r.proof_ref {
        out.extend_from_slice(b",\"proof_ref\":");
        push_str_json(out, p);
    }
    if let Some(j) = &r.justification {
        out.extend_from_slice(b",\"justification\":");
        push_str_json(out, j);
    }
    out.push(b'}');
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Parse and schema-validate a verdicts file. Fail-closed: a wrong schema
/// version, a semantics-version mismatch, an oversized or malformed file is an
/// error (E6402), never a silent best-effort read.
pub fn read_verdicts(bytes: &[u8]) -> Result<Verdicts, VerdictError> {
    read_echo(bytes).map(|e| e.verdicts)
}

/// Parse a langc verdicts echo (`<Module>.verdicts.inTree.json`), which is a
/// verdicts file plus the module's `stale_verdicts` count and `emitted`
/// check-accounting (FR-15). Same fail-closed validation as
/// [`read_verdicts`]; the echo-only keys default to zero when absent, so an
/// ordinary input file parses too.
pub fn read_echo(bytes: &[u8]) -> Result<Echo, VerdictError> {
    if bytes.len() > VERDICTS_FILE_MAX_BYTES {
        return Err(VerdictError::TooLarge { size: bytes.len() });
    }
    let mut r = VReader { b: bytes, i: 0 };
    let doc = r.parse_doc()?;
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
    })
}

/// The parsed verdicts echo (verdicts + stale count + emitted accounting).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Echo {
    pub verdicts: Verdicts,
    pub stale_verdicts: u32,
    pub emitted: EmittedChecksData,
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

    fn parse_doc(&mut self) -> Result<Doc, VerdictError> {
        self.skip_ws();
        if self.bump()? != b'{' {
            return self.err();
        }
        let mut schema: Option<String> = None;
        let mut semantics: Option<String> = None;
        let mut records: Option<Vec<VerdictRecord>> = None;
        let mut stale_verdicts: Option<u32> = None;
        let mut emitted: Option<EmittedChecksData> = None;
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
                "tool" => {
                    let _ = self.parse_tool()?;
                }
                "verdicts" => records = Some(self.parse_records()?),
                // Echo extras (P4): parsed here so typeu's report composition
                // can read them back; the strict `read_verdicts` input path
                // ignores them.
                "stale_verdicts" => stale_verdicts = Some(self.parse_u32()?),
                "emitted" => emitted = Some(self.parse_emitted()?),
                // Additive keys are skipped — readers tolerate future growth.
                _ => self.skip_value()?,
            }
        }
        let (Some(schema), Some(semantics), Some(records)) = (schema, semantics, records) else {
            return self.err();
        };
        let sem = semantics.clone();
        Ok(Doc {
            schema,
            semantics: sem.clone(),
            verdicts: Verdicts {
                semantics: sem,
                records,
            },
            stale_verdicts: stale_verdicts.unwrap_or(0),
            emitted: emitted.unwrap_or_default(),
        })
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

    fn parse_tool(&mut self) -> Result<(), VerdictError> {
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
            let _ = self.parse_string()?;
            self.expect(b':')?;
            self.skip_value()?;
        }
        Ok(())
    }

    fn parse_records(&mut self) -> Result<Vec<VerdictRecord>, VerdictError> {
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
            out.push(self.parse_record()?);
        }
        Ok(out)
    }

    fn parse_record(&mut self) -> Result<VerdictRecord, VerdictError> {
        self.expect(b'{')?;
        let mut id: Option<String> = None;
        let mut id_hash: Option<String> = None;
        let mut status: Option<VerdictStatus> = None;
        let mut method: Option<String> = None;
        let mut proof_ref: Option<String> = None;
        let mut justification: Option<String> = None;
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
                "id" => id = Some(self.parse_capped_string(VERDICT_ID_MAX_BYTES)?),
                "id_hash" => id_hash = Some(self.parse_capped_string(VERDICT_ID_MAX_BYTES)?),
                "status" => {
                    let s = self.parse_string()?;
                    status = VerdictStatus::from_file_str(&s);
                }
                "method" => method = Some(self.parse_string()?),
                "proof_ref" => proof_ref = Some(self.parse_string()?),
                "justification" => justification = Some(self.parse_string()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(id), Some(id_hash), Some(status)) = (id, id_hash, status) else {
            return self.err();
        };
        // An assumed verdict without a justification is not a decision (?);
        // the toolchain records what the file says either way — the policy
        // layer (--verify-policy=no-open-no-assumptions, E6410) surfaces it.
        Ok(VerdictRecord {
            id,
            id_hash,
            status,
            method,
            proof_ref,
            justification,
        })
    }

    /// Parse a string, enforcing the §7.5 id length cap.
    fn parse_capped_string(&mut self, cap: usize) -> Result<String, VerdictError> {
        let s = self.parse_string()?;
        if s.len() > cap {
            return Err(VerdictError::Malformed);
        }
        Ok(s)
    }

    /// Skip one JSON value (additive-schema tolerance).
    fn skip_value(&mut self) -> Result<(), VerdictError> {
        self.skip_ws();
        let c = self.bump()?;
        match c {
            b'{' => {
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        break;
                    }
                    if self.peek() == Some(b',') {
                        self.i += 1;
                        continue;
                    }
                    if self.peek().is_none() {
                        return self.err();
                    }
                    self.parse_string()?;
                    self.expect(b':')?;
                    self.skip_value()?;
                }
            }
            b'[' => {
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        break;
                    }
                    if self.peek() == Some(b',') {
                        self.i += 1;
                        continue;
                    }
                    if self.peek().is_none() {
                        return self.err();
                    }
                    self.skip_value()?;
                }
            }
            b'"' => {
                self.i -= 1;
                self.parse_string()?;
            }
            b't' | b'f' => {
                self.parse_bool()?;
            }
            b'n' => {
                if self.b.get(self.i..self.i + 4) == Some(b"null") {
                    self.i += 4;
                } else {
                    return self.err();
                }
            }
            b'-' | b'0'..=b'9' => {
                self.i -= 1;
                self.parse_i64()?;
            }
            _ => return self.err(),
        }
        Ok(())
    }

    /// Parse a JSON string (with escapes) into a Rust `String`.
    fn parse_string(&mut self) -> Result<String, VerdictError> {
        self.skip_ws();
        if self.bump()? != b'"' {
            return self.err();
        }
        let mut out = String::new();
        loop {
            let b = self.bump()?;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.bump()?;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let h = self.parse_hex4()?;
                            match core::char::from_u32(h) {
                                Some(c) => out.push(c),
                                None => return self.err(),
                            }
                        }
                        _ => return self.err(),
                    }
                }
                c if c < 0x20 => return self.err(),
                _ => {
                    let start = self.i - 1;
                    let b0 = self.b[start];
                    let len = match b0 {
                        0x00..=0x7F => 1,
                        0xC2..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF4 => 4,
                        _ => return self.err(),
                    };
                    if start + len > self.b.len() {
                        return self.err();
                    }
                    match core::str::from_utf8(&self.b[start..start + len]) {
                        Ok(s) => {
                            out.push_str(s);
                            self.i = start + len;
                        }
                        Err(_) => return self.err(),
                    }
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, VerdictError> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let b = self.bump()?;
            let d = match b {
                b'0'..=b'9' => (b - b'0') as u32,
                b'a'..=b'f' => (b - b'a' + 10) as u32,
                b'A'..=b'F' => (b - b'A' + 10) as u32,
                _ => return self.err(),
            };
            v = v * 16 + d;
        }
        Ok(v)
    }

    fn parse_bool(&mut self) -> Result<bool, VerdictError> {
        self.skip_ws();
        if self.b.get(self.i..self.i + 4) == Some(b"true") {
            self.i += 4;
            Ok(true)
        } else if self.b.get(self.i..self.i + 5) == Some(b"false") {
            self.i += 5;
            Ok(false)
        } else {
            self.err()
        }
    }

    fn parse_i64(&mut self) -> Result<i64, VerdictError> {
        self.skip_ws();
        let neg = self.peek() == Some(b'-');
        if neg {
            self.i += 1;
        }
        let mut v: i64 = 0;
        let mut digits = 0usize;
        while let Some(d) = self.peek() {
            if !d.is_ascii_digit() {
                break;
            }
            let d = (d - b'0') as i64;
            v = v
                .checked_mul(10)
                .and_then(|x| x.checked_add(d))
                .ok_or(VerdictError::Malformed)?;
            digits += 1;
            self.i += 1;
        }
        if digits == 0 {
            return self.err();
        }
        if neg {
            v = -v;
        }
        Ok(v)
    }
}

struct Doc {
    schema: String,
    semantics: String,
    verdicts: Verdicts,
    stale_verdicts: u32,
    emitted: EmittedChecksData,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn sample_records() -> Vec<VerdictRecord> {
        vec![
            VerdictRecord {
                id: "Bank::withdraw::subtype-range::0".to_string(),
                id_hash: "91cc0f2a4e7b18d3".to_string(),
                status: VerdictStatus::Discharged,
                method: Some("interval".to_string()),
                proof_ref: None,
                justification: None,
            },
            VerdictRecord {
                id: "Bank::counter::contract-post::0".to_string(),
                id_hash: "deadbeefdeadbeef".to_string(),
                status: VerdictStatus::Assumed,
                method: None,
                proof_ref: None,
                justification: Some("manual review 2026-09-19".to_string()),
            },
        ]
    }

    #[test]
    fn encode_decode_is_lossless_with_fixed_order() {
        let bytes = encode_verdicts("tyu-intervals", "0.1.0", &sample_records(), 1, &EmittedChecksData { subtype_range: 4, contract: 2, mmio_bounds: 0 })
            .expect("encode");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("{\"schema\":\"tyu.verdicts/v1\",\"tool\":{\"name\":\"tyu-intervals\""));
        // Unknown keys (echo extras) are skipped by the strict reader, which
        // must still validate schema + semantics and return the records.
        let parsed = read_verdicts(&bytes).expect("decode");
        assert_eq!(parsed.records, sample_records());
    }

    #[test]
    fn empty_file_is_valid() {
        let bytes = encode_verdicts("tyu", "0.1.0", &[], 0, &EmittedChecksData::default()).expect("encode");
        let parsed = read_verdicts(&bytes).expect("empty verdicts are valid");
        assert!(parsed.records.is_empty());
    }

    #[test]
    fn lookup_matches_id_and_hash_together() {
        let v = read_verdicts(
            &encode_verdicts("t", "0", &sample_records(), 0, &EmittedChecksData::default()).expect("encode"),
        )
        .expect("decode");
        assert!(v.lookup("Bank::withdraw::subtype-range::0", "91cc0f2a4e7b18d3").is_some());
        // Wrong hash for the same id — fail-closed (absent), never a match.
        assert!(v.lookup("Bank::withdraw::subtype-range::0", "0000000000000000").is_none());
        // Unknown id — absent.
        assert!(v.lookup("nope", "91cc0f2a4e7b18d3").is_none());
    }

    #[test]
    fn unknown_status_is_malformed() {
        let mut doc = String::from_utf8_lossy(
            &encode_verdicts("t", "0", &sample_records(), 0, &EmittedChecksData::default()).expect("encode"),
        )
        .into_owned();
        doc = doc.replace("\"status\":\"discharged\"", "\"status\":\"proven\"");
        assert_eq!(read_verdicts(doc.as_bytes()), Err(VerdictError::Malformed));
    }

    #[test]
    fn wrong_schema_and_semantics_are_hard_errors() {
        let mut doc = String::from_utf8_lossy(
            &encode_verdicts("t", "0", &[], 0, &EmittedChecksData::default()).expect("encode"),
        )
        .into_owned();
        doc = doc.replacen("tyu.verdicts/v1", "tyu.verdicts/v999", 1);
        assert!(matches!(
            read_verdicts(doc.as_bytes()),
            Err(VerdictError::SchemaVersion { .. })
        ));
        let mut doc2 = String::from_utf8_lossy(
            &encode_verdicts("t", "0", &[], 0, &EmittedChecksData::default()).expect("encode"),
        )
        .into_owned();
        doc2 = doc2.replacen(SEMANTICS_VERSION, "tyu.ir-sem/999.0", 1);
        assert!(matches!(
            read_verdicts(doc2.as_bytes()),
            Err(VerdictError::SemanticsMismatch { .. })
        ));
    }

    #[test]
    fn oversized_file_fails_closed() {
        let bytes = vec![b' '; VERDICTS_FILE_MAX_BYTES + 1];
        assert!(matches!(
            read_verdicts(&bytes),
            Err(VerdictError::TooLarge { .. })
        ));
    }

    #[test]
    fn stale_count_keeps_matched_and_counts_mismatched_and_unknown() {
        use crate::model::{canonical_id, fnv1a64, format_hex, ExtractionCtx, Formula, Kind, Oel, Provenance};
        let mut ctx = ExtractionCtx::new(b"Bank");
        ctx.begin_word(b"withdraw");
        ctx.record(
            Kind::SubtypeRange,
            Formula::InRange { value: Oel::Var { name: "in.0".to_string() }, lo: 0, hi: 100 },
            0,
            0,
            Provenance::Direct,
            Vec::new(),
        );
        let obligations = ctx.set().obligations.clone();
        // One matching record, one hash-mismatched, one totally unknown.
        let matching_id = obligations[0].id.clone();
        let matching_hash = obligations[0].id_hash.clone();
        let recs = vec![
            VerdictRecord {
                id: matching_id,
                id_hash: matching_hash,
                status: VerdictStatus::Discharged,
                method: Some("interval".to_string()),
                proof_ref: None,
                justification: None,
            },
            VerdictRecord {
                id: obligations[0].id.clone(),
                id_hash: "0000000000000000".to_string(),
                status: VerdictStatus::Assumed,
                method: None,
                proof_ref: None,
                justification: Some("stale".to_string()),
            },
            VerdictRecord {
                id: "Other::word::subtype-range::0".to_string(),
                id_hash: "1234567890abcdef".to_string(),
                status: VerdictStatus::Discharged,
                method: Some("interval".to_string()),
                proof_ref: None,
                justification: None,
            },
        ];
        assert_eq!(recs.len() as u32, 3);
        // Sanity: canonical id/hash pipeline is stable (Q3).
        assert_eq!(format_hex(fnv1a64(canonical_id("Bank", b"withdraw", Kind::SubtypeRange, 0).as_bytes())), obligations[0].id_hash);
        let v = Verdicts { semantics: SEMANTICS_VERSION.to_string(), records: recs };
        assert_eq!(v.stale_count(&obligations), 2);
    }
}