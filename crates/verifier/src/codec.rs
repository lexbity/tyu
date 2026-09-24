//! `.obl.json` codec (static-verification.md §6.1, slice P2).
//!
//! Hand-rolled JSON — deliberately no `serde`/`serde_json` dependency
//! (see the module doc of [`crate::model`] for the `-nodefaultlibs` link
//! constraint): the writer emits the fixed schema in fixed key order (Q11
//! determinism — no map anywhere, so no iteration-order surface), and the
//! reader is a strict, schema-specific parser that fail-closes on E6400
//! (schema version) / E6401 (malformed) and on semantics-version mismatch.
//!
//! `encode_obl`/`write_obl` are the single writer (used by `langc`);
//! `read_obl` is the schema-validating reader (used by consumers).

use crate::model::{
    Assumption, Facts, Formula, Kind, OblSet, Obligation, Oel, Provenance, Site, SpanInfo,
    SubtypeFact, WordFact, OBL_ARTIFACT_MAX_BYTES, OBL_SCHEMA,
};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// Codec failure classes. The reader side maps onto the artifact-band
/// diagnostics E6400 (schema version) / E6401 (malformed) — see
/// static-verification.md FR-19 and the error-registry appendix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    /// Not parseable as the artifact shape at all.
    Malformed,
    /// Top-level `schema` is not `tyu.obl/v1` (E6400).
    SchemaVersion { found: String },
    /// Top-level `semantics` disagrees with `SEMANTICS_VERSION` (a newer/older
    /// op-semantics row set than this toolchain's — verdict caches are keyed
    /// on it, Q3).
    SemanticsMismatch { found: String },
    /// Artifact exceeds the 16 MiB read-side cap (NFR-5) — fail closed.
    TooLarge { size: usize },
}

impl CodecError {
    /// The diagnostic code the fail-closed paths surface (claims registry +
    /// error-registry appendix).
    pub fn code(&self) -> u32 {
        match self {
            CodecError::SchemaVersion { .. } => 6400,
            _ => 6401,
        }
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Serialize a complete obligation set to JSON bytes (fixed key order, Q11).
/// Enforces the 16 MiB cap on the write side too (NFR-5).
pub fn encode_obl(set: &OblSet) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::with_capacity(1024);
    write_doc(&mut out, set);
    if out.len() > OBL_ARTIFACT_MAX_BYTES {
        return Err(CodecError::TooLarge { size: out.len() });
    }
    Ok(out)
}

/// Write a complete obligation set into an `ir::Output` sink.
/// Returns the number of bytes written.
pub fn write_obl(out: &mut dyn ir::Output, set: &OblSet) -> Result<usize, CodecError> {
    let bytes = encode_obl(set)?;
    out.write(&bytes);
    Ok(bytes.len())
}

fn write_doc(out: &mut Vec<u8>, set: &OblSet) {
    out.extend_from_slice(b"{\"schema\":");
    write_str(out, &set.schema);
    out.extend_from_slice(b",\"semantics\":");
    write_str(out, &set.semantics);
    out.extend_from_slice(b",\"module\":");
    write_str(out, &set.module);
    out.extend_from_slice(b",\"abi_contract_version\":");
    write_i64(out, set.abi_contract_version as i64);
    write_facts(out, &set.facts);
    out.extend_from_slice(b",\"obligations\":[");
    for (i, o) in set.obligations.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        write_obligation(out, o);
    }
    out.extend_from_slice(b"]}");
}

fn write_facts(out: &mut Vec<u8>, facts: &Facts) {
    out.extend_from_slice(b",\"facts\":{\"words\":[");
    for (i, w) in facts.words.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"name\":");
        write_str(out, &w.name);
        out.extend_from_slice(b",\"net\":");
        write_i64(out, w.net as i64);
        out.extend_from_slice(b",\"high\":");
        write_i64(out, w.high as i64);
        out.extend_from_slice(b",\"top\":");
        out.extend_from_slice(if w.top { b"true" } else { b"false" });
        out.extend_from_slice(b",\"performs\":[");
        for (j, p) in w.performs.iter().enumerate() {
            if j != 0 {
                out.push(b',');
            }
            write_str(out, p);
        }
        out.extend_from_slice(b"],\"diverge_free\":");
        out.extend_from_slice(if w.diverge_free { b"true" } else { b"false" });
        out.push(b'}');
    }
    out.extend_from_slice(b"],\"subtypes\":[");
    for (i, s) in facts.subtypes.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"name\":");
        write_str(out, &s.name);
        out.extend_from_slice(b",\"lo\":");
        write_i64(out, s.lo);
        out.extend_from_slice(b",\"hi\":");
        write_i64(out, s.hi);
        out.push(b'}');
    }
    out.extend_from_slice(b"]}");
}

fn write_obligation(out: &mut Vec<u8>, o: &Obligation) {
    out.extend_from_slice(b"{\"id\":");
    write_str(out, &o.id);
    out.extend_from_slice(b",\"id_hash\":");
    write_str(out, &o.id_hash);
    out.extend_from_slice(b",\"kind\":");
    write_str(out, o.kind.as_str());
    out.extend_from_slice(b",\"site\":{\"word\":");
    write_str(out, &o.site.word);
    out.extend_from_slice(b",\"occurrence\":");
    write_i64(out, o.site.occurrence as i64);
    out.extend_from_slice(b",\"span\":{\"line\":");
    write_i64(out, o.site.span.line as i64);
    out.extend_from_slice(b",\"col\":");
    write_i64(out, o.site.span.col as i64);
    out.extend_from_slice(b"}},\"formula\":");
    write_formula(out, &o.formula);
    out.extend_from_slice(b",\"assumptions\":[");
    for (i, a) in o.assumptions.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        write_assumption(out, a);
    }
    out.extend_from_slice(b"],\"provenance\":");
    write_str(out, o.provenance.as_str());
    out.push(b'}');
}

fn write_formula(out: &mut Vec<u8>, f: &Formula) {
    match f {
        Formula::InRange { value, lo, hi } => {
            out.extend_from_slice(b"{\"op\":\"InRange\",\"value\":");
            write_oel(out, value);
            out.extend_from_slice(b",\"lo\":");
            write_i64(out, *lo);
            out.extend_from_slice(b",\"hi\":");
            write_i64(out, *hi);
            out.push(b'}');
        }
        Formula::OffsetLE { off, width, size } => {
            out.extend_from_slice(b"{\"op\":\"OffsetLE\",\"off\":");
            match off {
                Some(off) => write_i64(out, *off as i64),
                None => out.extend_from_slice(b"null"),
            }
            out.extend_from_slice(b",\"width\":");
            write_i64(out, *width as i64);
            out.extend_from_slice(b",\"size\":");
            write_i64(out, *size as i64);
            out.push(b'}');
        }
    }
}

fn write_oel(out: &mut Vec<u8>, v: &Oel) {
    match v {
        Oel::Var { name } => {
            out.extend_from_slice(b"{\"op\":\"Var\",\"name\":");
            write_str(out, name);
            out.push(b'}');
        }
        Oel::Cast { from, to, arg } => {
            out.extend_from_slice(b"{\"op\":\"Cast\",\"from\":");
            write_str(out, from);
            out.extend_from_slice(b",\"to\":");
            write_str(out, to);
            out.extend_from_slice(b",\"arg\":");
            write_oel(out, arg);
            out.push(b'}');
        }
    }
}

fn write_assumption(out: &mut Vec<u8>, a: &Assumption) {
    match a {
        Assumption::SubtypeRange { name, lo, hi } => {
            out.extend_from_slice(b"{\"kind\":\"subtype-range\",\"name\":");
            write_str(out, name);
            out.extend_from_slice(b",\"lo\":");
            write_i64(out, *lo);
            out.extend_from_slice(b",\"hi\":");
            write_i64(out, *hi);
            out.push(b'}');
        }
        Assumption::ApertureSize { aperture, size } => {
            out.extend_from_slice(b"{\"kind\":\"aperture-size\",\"aperture\":");
            write_i64(out, *aperture as i64);
            out.extend_from_slice(b",\"size\":");
            write_i64(out, *size as i64);
            out.push(b'}');
        }
    }
}

/// Write `s` as a JSON string with escaping. Identifiers are ASCII in
/// practice; this handles the full escape surface anyway.
pub(crate) fn push_str_json(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for &b in s.as_bytes() {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0C => out.extend_from_slice(b"\\f"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x00..=0x1F => {
                out.extend_from_slice(b"\\u00");
                let h = format_hex2(b);
                out.extend_from_slice(h.as_bytes());
            }
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    push_str_json(out, s);
}

/// Two lowercase hex digits for a byte (control-char JSON escapes).
fn format_hex2(b: u8) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(2);
    s.push(DIGITS[(b >> 4) as usize] as char);
    s.push(DIGITS[(b & 0xF) as usize] as char);
    s
}

/// Write `v` as a JSON integer (decimal; negative allowed).
pub(crate) fn push_i64_json(out: &mut Vec<u8>, mut v: i64) {
    if v == i64::MIN {
        out.extend_from_slice(b"-9223372036854775808");
        return;
    }
    if v < 0 {
        out.push(b'-');
        v = -v;
    }
    // up to 20 digits
    let mut buf = [0u8; 20];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    for i in (0..n).rev() {
        out.push(buf[i]);
    }
}

fn write_i64(out: &mut Vec<u8>, v: i64) {
    push_i64_json(out, v);
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// Parse and schema-validate an `.obl.json` artifact. Fail-closed: a wrong
/// schema version, a semantics-version mismatch, or an oversized artifact is
/// an error (E6400/E6401), never a silent best-effort read.
pub fn read_obl(bytes: &[u8]) -> Result<OblSet, CodecError> {
    if bytes.len() > OBL_ARTIFACT_MAX_BYTES {
        return Err(CodecError::TooLarge { size: bytes.len() });
    }
    let mut r = Reader { b: bytes, i: 0 };
    let set = r.parse_doc()?;
    if set.schema != OBL_SCHEMA {
        return Err(CodecError::SchemaVersion { found: set.schema });
    }
    if set.semantics != crate::semantics::SEMANTICS_VERSION {
        return Err(CodecError::SemanticsMismatch {
            found: set.semantics,
        });
    }
    Ok(set)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

/// Length in bytes of the UTF-8 sequence starting with `b0` (1..=4), per the
/// standard leading-byte layout; 0 means the byte is not a valid lead byte.
fn utf8_seq_len(b0: u8) -> usize {
    match b0 {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 0,
    }
}

impl<'a> Reader<'a> {
    fn err<T>(&self) -> Result<T, CodecError> {
        Err(CodecError::Malformed)
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn bump(&mut self) -> Result<u8, CodecError> {
        let b = self.b.get(self.i).copied().ok_or(CodecError::Malformed)?;
        self.i += 1;
        Ok(b)
    }

    fn expect(&mut self, want: u8) -> Result<(), CodecError> {
        self.skip_ws();
        if self.bump()? != want {
            self.err()
        } else {
            Ok(())
        }
    }

    fn parse_doc(&mut self) -> Result<OblSet, CodecError> {
        self.skip_ws();
        if self.bump()? != b'{' {
            return self.err();
        }
        let mut schema: Option<String> = None;
        let mut semantics: Option<String> = None;
        let mut module: Option<String> = None;
        let mut abi_contract_version: Option<u32> = None;
        let mut facts: Option<Facts> = None;
        let mut obligations: Option<Vec<Obligation>> = None;
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
                "module" => module = Some(self.parse_string()?),
                "abi_contract_version" => abi_contract_version = Some(self.parse_u64()? as u32),
                "facts" => facts = Some(self.parse_facts()?),
                "obligations" => obligations = Some(self.parse_obligations()?),
                // Unknown keys are skipped — additive schema growth (owner doc §6).
                _ => self.skip_value()?,
            }
        }
        let (Some(schema), Some(semantics), Some(module), Some(abi_contract_version), Some(facts), Some(obligations)) =
            (schema, semantics, module, abi_contract_version, facts, obligations)
        else {
            return self.err();
        };
        Ok(OblSet {
            schema,
            semantics,
            module,
            abi_contract_version,
            facts,
            obligations,
        })
    }

    fn parse_facts(&mut self) -> Result<Facts, CodecError> {
        self.expect(b'{')?;
        let mut words: Option<Vec<WordFact>> = None;
        let mut subtypes: Option<Vec<SubtypeFact>> = None;
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
                "words" => words = Some(self.parse_word_facts()?),
                "subtypes" => subtypes = Some(self.parse_subtype_facts()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(words), Some(subtypes)) = (words, subtypes) else {
            return self.err();
        };
        Ok(Facts { words, subtypes })
    }

    fn parse_word_facts(&mut self) -> Result<Vec<WordFact>, CodecError> {
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
            let mut name: Option<String> = None;
            let mut net: Option<i16> = None;
            let mut high: Option<u32> = None;
            let mut top: Option<bool> = None;
            let mut performs: Option<Vec<String>> = None;
            let mut diverge_free: Option<bool> = None;
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
                    "name" => name = Some(self.parse_string()?),
                    "net" => net = Some(self.parse_i64()? as i16),
                    "high" => high = Some(self.parse_u64()? as u32),
                    "top" => top = Some(self.parse_bool()?),
                    "performs" => performs = Some(self.parse_string_array()?),
                    "diverge_free" => diverge_free = Some(self.parse_bool()?),
                    _ => self.skip_value()?,
                }
            }
            let (Some(name), Some(net), Some(high), Some(top), Some(performs), Some(diverge_free)) =
                (name, net, high, top, performs, diverge_free)
            else {
                return self.err();
            };
            out.push(WordFact {
                name,
                net,
                high,
                top,
                performs,
                diverge_free,
            });
        }
        Ok(out)
    }

    fn parse_subtype_facts(&mut self) -> Result<Vec<SubtypeFact>, CodecError> {
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
            let mut name: Option<String> = None;
            let mut lo: Option<i64> = None;
            let mut hi: Option<i64> = None;
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
                    "name" => name = Some(self.parse_string()?),
                    "lo" => lo = Some(self.parse_i64()?),
                    "hi" => hi = Some(self.parse_i64()?),
                    _ => self.skip_value()?,
                }
            }
            let (Some(name), Some(lo), Some(hi)) = (name, lo, hi) else {
                return self.err();
            };
            out.push(SubtypeFact { name, lo, hi });
        }
        Ok(out)
    }

    fn parse_obligations(&mut self) -> Result<Vec<Obligation>, CodecError> {
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
            out.push(self.parse_obligation()?);
        }
        Ok(out)
    }

    fn parse_obligation(&mut self) -> Result<Obligation, CodecError> {
        self.expect(b'{')?;
        let mut id: Option<String> = None;
        let mut id_hash: Option<String> = None;
        let mut kind: Option<Kind> = None;
        let mut site: Option<Site> = None;
        let mut formula: Option<Formula> = None;
        let mut assumptions: Option<Vec<Assumption>> = None;
        let mut provenance: Option<Provenance> = None;
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
                "kind" => {
                    let k = self.parse_string()?;
                    kind = Kind::from_str(&k);
                }
                "site" => site = Some(self.parse_site()?),
                "formula" => formula = Some(self.parse_formula()?),
                "assumptions" => assumptions = Some(self.parse_assumptions()?),
                "provenance" => {
                    let p = self.parse_string()?;
                    provenance = Provenance::from_str(&p);
                }
                _ => self.skip_value()?,
            }
        }
        let (Some(id), Some(id_hash), Some(kind), Some(site), Some(formula), Some(assumptions), Some(provenance)) =
            (id, id_hash, kind, site, formula, assumptions, provenance)
        else {
            return self.err();
        };
        Ok(Obligation {
            id,
            id_hash,
            kind,
            site,
            formula,
            assumptions,
            provenance,
        })
    }

    fn parse_site(&mut self) -> Result<Site, CodecError> {
        self.expect(b'{')?;
        let mut word: Option<String> = None;
        let mut occurrence: Option<u32> = None;
        let mut span: Option<SpanInfo> = None;
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
                "word" => word = Some(self.parse_string()?),
                "occurrence" => occurrence = Some(self.parse_u64()? as u32),
                "span" => span = Some(self.parse_span()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(word), Some(occurrence), Some(span)) = (word, occurrence, span) else {
            return self.err();
        };
        Ok(Site { word, occurrence, span })
    }

    fn parse_span(&mut self) -> Result<SpanInfo, CodecError> {
        self.expect(b'{')?;
        let mut line: Option<u32> = None;
        let mut col: Option<u32> = None;
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
                "line" => line = Some(self.parse_u64()? as u32),
                "col" => col = Some(self.parse_u64()? as u32),
                _ => self.skip_value()?,
            }
        }
        let (Some(line), Some(col)) = (line, col) else {
            return self.err();
        };
        Ok(SpanInfo { line, col })
    }

    fn parse_formula(&mut self) -> Result<Formula, CodecError> {
        self.expect(b'{')?;
        let mut op: Option<String> = None;
        let mut value: Option<Oel> = None;
        let mut lo: Option<i64> = None;
        let mut hi: Option<i64> = None;
        let mut off: Option<Option<u32>> = None;
        let mut width: Option<u32> = None;
        let mut size: Option<u32> = None;
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
                "op" => op = Some(self.parse_string()?),
                "value" => value = Some(self.parse_oel()?),
                "lo" => lo = Some(self.parse_i64()?),
                "hi" => hi = Some(self.parse_i64()?),
                "off" => off = Some(self.parse_nullable_u32()?),
                "width" => width = Some(self.parse_u64()? as u32),
                "size" => size = Some(self.parse_u64()? as u32),
                _ => self.skip_value()?,
            }
        }
        let Some(op) = op else { return self.err() };
        match op.as_str() {
            "InRange" => {
                let (Some(value), Some(lo), Some(hi)) = (value, lo, hi) else {
                    return self.err();
                };
                Ok(Formula::InRange { value, lo, hi })
            }
            "OffsetLE" => {
                let (Some(off), Some(width), Some(size)) = (off, width, size) else {
                    return self.err();
                };
                Ok(Formula::OffsetLE { off, width, size })
            }
            _ => self.err(),
        }
    }

    /// `null` (dynamic offset) or a number.
    fn parse_nullable_u32(&mut self) -> Result<Option<u32>, CodecError> {
        self.skip_ws();
        if self.peek() == Some(b'n') {
            if self.b.get(self.i..self.i + 4) == Some(b"null") {
                self.i += 4;
                return Ok(None);
            }
            return self.err();
        }
        self.parse_u64().map(|v| Some(v as u32))
    }

    fn parse_oel(&mut self) -> Result<Oel, CodecError> {
        self.expect(b'{')?;
        let mut op: Option<String> = None;
        let mut name: Option<String> = None;
        let mut from: Option<String> = None;
        let mut to: Option<String> = None;
        let mut arg: Option<Oel> = None;
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
                "op" => op = Some(self.parse_string()?),
                "name" => name = Some(self.parse_string()?),
                "from" => from = Some(self.parse_string()?),
                "to" => to = Some(self.parse_string()?),
                "arg" => arg = Some(self.parse_oel()?),
                _ => self.skip_value()?,
            }
        }
        match op.as_deref() {
            Some("Var") => {
                let Some(name) = name else { return self.err() };
                Ok(Oel::Var { name })
            }
            Some("Cast") => {
                let (Some(from), Some(to), Some(arg)) = (from, to, arg) else {
                    return self.err();
                };
                Ok(Oel::Cast {
                    from,
                    to,
                    arg: Box::new(arg),
                })
            }
            _ => self.err(),
        }
    }

    fn parse_assumptions(&mut self) -> Result<Vec<Assumption>, CodecError> {
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
            out.push(self.parse_assumption()?);
        }
        Ok(out)
    }

    fn parse_assumption(&mut self) -> Result<Assumption, CodecError> {
        self.expect(b'{')?;
        let mut kind: Option<String> = None;
        let mut name: Option<String> = None;
        let mut lo: Option<i64> = None;
        let mut hi: Option<i64> = None;
        let mut aperture: Option<u16> = None;
        let mut size: Option<u32> = None;
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
                "kind" => kind = Some(self.parse_string()?),
                "name" => name = Some(self.parse_string()?),
                "lo" => lo = Some(self.parse_i64()?),
                "hi" => hi = Some(self.parse_i64()?),
                "aperture" => aperture = Some(self.parse_u64()? as u16),
                "size" => size = Some(self.parse_u64()? as u32),
                _ => self.skip_value()?,
            }
        }
        let Some(kind) = kind else { return self.err() };
        match kind.as_str() {
            "subtype-range" => {
                let (Some(name), Some(lo), Some(hi)) = (name, lo, hi) else {
                    return self.err();
                };
                Ok(Assumption::SubtypeRange { name, lo, hi })
            }
            "aperture-size" => {
                let (Some(aperture), Some(size)) = (aperture, size) else {
                    return self.err();
                };
                Ok(Assumption::ApertureSize { aperture, size })
            }
            _ => self.err(),
        }
    }

    fn parse_string_array(&mut self) -> Result<Vec<String>, CodecError> {
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
            out.push(self.parse_string()?);
        }
        Ok(out)
    }

    /// Skip one JSON value (additive-schema tolerance).
    fn skip_value(&mut self) -> Result<(), CodecError> {
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
    fn parse_string(&mut self) -> Result<String, CodecError> {
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
                    // Consume exactly one UTF-8 sequence (no alloc machinery).
                    let start = self.i - 1;
                    let b0 = self.b[start];
                    let len = utf8_seq_len(b0);
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

    fn parse_hex4(&mut self) -> Result<u32, CodecError> {
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

    fn parse_bool(&mut self) -> Result<bool, CodecError> {
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

    fn parse_i64(&mut self) -> Result<i64, CodecError> {
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
                .ok_or(CodecError::Malformed)?;
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

    fn parse_u64(&mut self) -> Result<u64, CodecError> {
        self.skip_ws();
        let mut v: u64 = 0;
        let mut digits = 0usize;
        while let Some(d) = self.peek() {
            if !d.is_ascii_digit() {
                break;
            }
            let d = (d - b'0') as u64;
            v = v.checked_mul(10).and_then(|x| x.checked_add(d)).ok_or(CodecError::Malformed)?;
            digits += 1;
            self.i += 1;
        }
        if digits == 0 {
            return self.err();
        }
        Ok(v)
    }
}

// ---------------------------------------------------------------------------
// Report writer (static-verification.md §6.5, slice P3)
// ---------------------------------------------------------------------------

/// Serialize a `VerifyReport` to JSON bytes (fixed key order — §6.5 field
/// order; deterministic, FR-17). `tyu` builds the model; this is the single
/// report serializer.
pub fn encode_report(report: &crate::report::VerifyReport) -> Result<Vec<u8>, CodecError> {
    let out = write_report_bytes(report);
    if out.len() > OBL_ARTIFACT_MAX_BYTES {
        return Err(CodecError::TooLarge { size: out.len() });
    }
    Ok(out)
}

fn write_report_bytes(r: &crate::report::VerifyReport) -> Vec<u8> {
    let mut out = Vec::with_capacity(2048);
    out.extend_from_slice(b"{\"schema\":");
    write_str(&mut out, &r.schema);
    out.extend_from_slice(b",\"tool\":{\"name\":");
    write_str(&mut out, &r.tool.name);
    out.extend_from_slice(b",\"version\":");
    write_str(&mut out, &r.tool.version);
    out.extend_from_slice(b"},\"semantics\":");
    write_str(&mut out, &r.semantics);
    out.extend_from_slice(b",\"policy\":");
    write_str(&mut out, &r.policy);
    write_report_modules(&mut out, &r.modules);
    write_report_contexts(&mut out, &r.contexts);
    out.extend_from_slice(b",\"open\":[");
    for (i, o) in r.open.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"id\":");
        write_str(&mut out, &o.id);
        out.extend_from_slice(b",\"kind\":");
        write_str(&mut out, &o.kind);
        out.extend_from_slice(b",\"module\":");
        write_str(&mut out, &o.module);
        out.extend_from_slice(b",\"word\":");
        write_str(&mut out, &o.word);
        out.extend_from_slice(b",\"site\":");
        write_str(&mut out, &o.site);
        out.extend_from_slice(b",\"line\":");
        write_i64(&mut out, o.line as i64);
        out.push(b'}');
    }
    out.extend_from_slice(b"],\"assumed\":[");
    for (i, a) in r.assumed.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"id\":");
        write_str(&mut out, &a.id);
        out.extend_from_slice(b",\"kind\":");
        write_str(&mut out, &a.kind);
        out.extend_from_slice(b",\"module\":");
        write_str(&mut out, &a.module);
        out.extend_from_slice(b",\"word\":");
        write_str(&mut out, &a.word);
        out.extend_from_slice(b",\"site\":");
        write_str(&mut out, &a.site);
        out.extend_from_slice(b",\"line\":");
        write_i64(&mut out, a.line as i64);
        if let Some(j) = &a.justification {
            out.extend_from_slice(b",\"justification\":");
            write_str(&mut out, j);
        }
        out.push(b'}');
    }
    out.extend_from_slice(b"],\"assumptions_trusted\":[");
    for (i, a) in r.assumptions_trusted.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"kind\":");
        write_str(&mut out, &a.kind);
        out.extend_from_slice(b",\"what\":");
        write_str(&mut out, &a.what);
        out.push(b'}');
    }
    out.extend_from_slice(b"],\"stale_verdicts\":");
    write_i64(&mut out, r.stale_verdicts as i64);
    out.extend_from_slice(b",\"emitted_checks\":{\"subtype_range\":");
    write_i64(&mut out, r.emitted_checks.subtype_range as i64);
    out.extend_from_slice(b",\"contract\":");
    write_i64(&mut out, r.emitted_checks.contract as i64);
    out.extend_from_slice(b",\"mmio_bounds\":");
    write_i64(&mut out, r.emitted_checks.mmio_bounds as i64);
    out.extend_from_slice(b",\"data_stack_guards\":");
    out.extend_from_slice(if r.emitted_checks.data_stack_guards {
        b"true"
    } else {
        b"false"
    });
    out.extend_from_slice(b"}}");
    out
}

fn write_report_modules(out: &mut Vec<u8>, modules: &[crate::report::ModuleAccounting]) {
    out.extend_from_slice(b",\"modules\":[");
    for (i, m) in modules.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"name\":");
        write_str(out, &m.name);
        out.extend_from_slice(b",\"classes\":{");
        for (j, c) in m.classes.iter().enumerate() {
            if j != 0 {
                out.push(b',');
            }
            write_str(out, &c.kind);
            out.extend_from_slice(b":{\"total\":");
            write_i64(out, c.total as i64);
            out.extend_from_slice(b",\"discharged\":");
            write_i64(out, c.discharged as i64);
            out.extend_from_slice(b",\"assumed\":");
            write_i64(out, c.assumed as i64);
            out.extend_from_slice(b",\"open\":");
            write_i64(out, c.open as i64);
            out.push(b'}');
        }
        out.extend_from_slice(b"}}");
    }
    out.extend_from_slice(b"]");
}

fn write_report_contexts(out: &mut Vec<u8>, c: &crate::report::StackContextAccounting) {
    out.extend_from_slice(b",\"contexts\":{\"stack\":{\"main\":{\"high\":");
    write_i64(out, c.main.high as i64);
    out.extend_from_slice(b",\"top\":");
    out.extend_from_slice(if c.main.top { b"true" } else { b"false" });
    out.extend_from_slice(b",\"budget\":");
    write_i64(out, c.main.budget as i64);
    out.extend_from_slice(b",\"verdict\":");
    write_str(out, &c.main.verdict);
    out.extend_from_slice(b"},\"isr\":{\"max_high\":");
    write_i64(out, c.isr.max_high as i64);
    out.extend_from_slice(b",\"budget\":");
    write_i64(out, c.isr.budget as i64);
    out.extend_from_slice(b",\"handlers\":");
    write_i64(out, c.isr.handlers as i64);
    out.extend_from_slice(b",\"verdict\":");
    write_str(out, &c.isr.verdict);
    out.extend_from_slice(b"},\"guards\":");
    write_str(out, &c.guards);
    out.extend_from_slice(b"}}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        canonical_id, fnv1a64, format_hex, ExtractionCtx, Formula, Kind, Oel, Provenance,
    };
    use alloc::string::ToString;
    use alloc::vec;

    fn sample_set() -> OblSet {
        let mut ctx = ExtractionCtx::new(b"Bank");
        ctx.begin_word(b"clamp");
        let mut assumptions = Vec::new();
        assumptions.push(crate::model::Assumption::ApertureSize {
            aperture: 0,
            size: 65536,
        });
        ctx.record(
            Kind::SubtypeRange,
            Formula::InRange {
                value: Oel::Var {
                    name: "$top".to_string(),
                },
                lo: 0,
                hi: 100,
            },
            12,
            8,
            Provenance::Opaque,
            assumptions,
        );
        ctx.push_subtype_fact(b"Percent", 0, 100);
        // An emulated-aperture access (P3): OffsetLE with a const offset.
        ctx.begin_word(b"read");
        let mut ctx = ctx;
        let mut mmio_assumptions = Vec::new();
        mmio_assumptions.push(crate::model::Assumption::ApertureSize {
            aperture: 0,
            size: 65536,
        });
        ctx.record(
            Kind::MmioBounds,
            Formula::OffsetLE {
                off: Some(0x1000),
                width: 4,
                size: 65536,
            },
            4,
            10,
            Provenance::Direct,
            mmio_assumptions,
        );
        ctx.set().clone()
    }

    /// Cross-check the hand-rolled writer against a reference parser
    /// (serde_json is a dev-dependency only — never in the lib graph).
    fn reference_parse(bytes: &[u8]) -> serde_json::Value {
        serde_json::from_slice(bytes).expect("hand-rolled writer must emit valid JSON")
    }

    #[test]
    fn encode_is_fixed_key_order_no_maps() {
        let set = sample_set();
        let bytes = encode_obl(&set).expect("encode");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("{\"schema\":\"tyu.obl/v1\",\"semantics\":\"tyu.ir-sem/1.0\",\"module\":\"Bank\",\"abi_contract_version\":1,\"facts\""));
        assert!(text.contains("\"obligations\":[{\"id\":\"Bank::clamp::subtype-range::0\",\"id_hash\":\""));
        assert!(text.contains("\"provenance\":\"opaque\""));
    }

    #[test]
    fn writer_emits_valid_json_reference_parsed() {
        let set = sample_set();
        let bytes = encode_obl(&set).expect("encode");
        let v = reference_parse(&bytes);
        assert_eq!(v["module"], "Bank");
        assert_eq!(v["obligations"][0]["id"], "Bank::clamp::subtype-range::0");
        assert_eq!(v["obligations"][0]["formula"]["op"], "InRange");
    }

    #[test]
    fn encode_decode_encode_is_byte_exact() {
        let set = sample_set();
        let first = encode_obl(&set).expect("encode");
        let decoded = read_obl(&first).expect("decode");
        assert_eq!(decoded, set, "decode must reproduce the model");
        let second = encode_obl(&decoded).expect("re-encode");
        assert_eq!(first, second, "encode -> decode -> encode must be byte-exact");
    }

    #[test]
    fn read_rejects_wrong_schema() {
        let set = sample_set();
        let mut doc: String = String::from_utf8_lossy(&encode_obl(&set).expect("encode"))
            .into_owned();
        doc = doc.replacen("tyu.obl/v1", "tyu.obl/v999", 1);
        assert_eq!(
            read_obl(doc.as_bytes()),
            Err(CodecError::SchemaVersion {
                found: "tyu.obl/v999".to_string()
            })
        );
    }

    #[test]
    fn read_rejects_malformed() {
        assert_eq!(read_obl(b"not json"), Err(CodecError::Malformed));
        assert_eq!(
            read_obl(b"{\"schema\": \"tyu.obl/v1\""),
            Err(CodecError::Malformed)
        );
    }

    #[test]
    fn read_rejects_oversized() {
        let big = vec![b' '; OBL_ARTIFACT_MAX_BYTES + 1];
        assert!(matches!(read_obl(&big), Err(CodecError::TooLarge { .. })));
    }

    #[test]
    fn kind_strings_parse_roundtrip() {
        for k in [Kind::SubtypeRange, Kind::ContractPre, Kind::ContractPost, Kind::StackBudget, Kind::MmioBounds] {
            assert_eq!(Kind::from_str(k.as_str()), Some(k));
        }
        assert_eq!(Kind::from_str("bogus"), None);
        assert_eq!(Provenance::from_str("direct"), Some(Provenance::Direct));
        assert_eq!(Provenance::from_str("opaque"), Some(Provenance::Opaque));
        assert_eq!(Provenance::from_str("bogus"), None);
    }

    #[test]
    fn fnv1a64_standard_vectors() {
        // Standard FNV-1a 64-bit parameters; `hello` cross-pins against
        // lmod::hash's authoritative test (`crates/lmod/src/hash.rs`), the
        // ABI symbol hash the loader consumes — equal algorithms, equal bytes.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a64(b"hello"), 0xa430_d846_80aa_bd0b);
        assert_eq!(format_hex(fnv1a64(b"")), "cbf29ce484222325");
        assert_eq!(format_hex(fnv1a64(b"hello")), "a430d84680aabd0b");
    }

    #[test]
    fn canonical_id_shape() {
        assert_eq!(
            canonical_id("Bank", b"withdraw", Kind::SubtypeRange, 0),
            "Bank::withdraw::subtype-range::0"
        );
        assert_eq!(
            canonical_id("Bank", b"clamp", Kind::SubtypeRange, 7),
            "Bank::clamp::subtype-range::7"
        );
    }

    /// The reader must skip unknown (future/additive) keys without failing.
    #[test]
    fn reader_tolerates_additive_keys() {
        let set = sample_set();
        let mut doc: String = String::from_utf8_lossy(&encode_obl(&set).expect("encode"))
            .into_owned();
        // Insert a future-style key inside the top-level object.
        doc = doc.replace(",\"facts\"", ",\"future_extra\":{\"a\":1},\"facts\"");
        let parsed = read_obl(doc.as_bytes()).expect("additive keys are skipped");
        assert_eq!(parsed.module, "Bank");
        assert_eq!(parsed.obligations.len(), 2);
    }
}