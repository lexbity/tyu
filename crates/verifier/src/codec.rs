//! `.obl.json` codec (static-verification.md §6.1, slice P2; PLAN-VERIFY-3
//! §6.1, P1.2 — `tyu.obl/v2`).
//!
//! Hand-rolled JSON — deliberately no `serde`/`serde_json` dependency
//! (see the module doc of [`crate::model`] for the `-nodefaultlibs` link
//! constraint): the writer emits the fixed schema in fixed key order (Q11
//! determinism — no map anywhere, so no iteration-order surface), and the
//! reader is a strict, schema-specific parser that fail-closes on E6400
//! (schema version) / E6401 (malformed) and on semantics-version mismatch.
//!
//! v2 (PLAN-VERIFY-3 P1.2) adds the statement-schema stamp, the `(target,
//! platform, model_semantics)` identity fields, per-word IR text + block
//! count (8 KiB per-word cap, `WordIrTooLarge`), and per-obligation `intent`
//! (§Q17), `assumptions` — the §Q7 dependency edges — and `cycles` (§Q9).
//! The v1 trusted-facts `assumptions` member is gone: every fact it carried
//! is derivable from the formula. The v1→v2 break is owned by §13: a v1
//! artifact fails E6400.
//!
//! `encode_obl`/`write_obl` are the single writer (used by `langc`);
//! `read_obl` is the schema-validating reader (used by consumers).

use crate::model::{
    Facts, Formula, Kind, OblSet, Obligation, Oel, PredicateFact, PredicateRef, Provenance, Site,
    SpanInfo, SubtypeFact, WordFact, OBL_ARTIFACT_MAX_BYTES, OBL_SCHEMA, WORD_IR_MAX_BYTES,
};
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Codec failure classes. The reader side maps onto the artifact-band
/// diagnostics E6400 (schema version) / E6401 (malformed) — see
/// static-verification.md FR-19 and the error-registry appendix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    /// Not parseable as the artifact shape at all.
    Malformed,
    /// Top-level `schema` is not `tyu.obl/v2` (E6400).
    SchemaVersion { found: String },
    /// Top-level `semantics` disagrees with `SEMANTICS_VERSION` (a newer/older
    /// op-semantics row set than this toolchain's — verdict caches are keyed
    /// on it, Q3).
    SemanticsMismatch { found: String },
    /// Artifact exceeds the 16 MiB read-side cap (NFR-5) — fail closed.
    TooLarge { size: usize },
    /// P1.2 (PLAN-VERIFY-3): a word's canonical op-text exceeds the 8 KiB
    /// per-word IR cap — fail-closed at encode (E6401-class), so no artifact
    /// ever ships word IR the statement pipeline would have to make sense of
    /// past its bound.
    WordIrTooLarge { word: String, size: usize },
    /// Slice P7: the image-verdicts record (E6415) — the guard-elision
    /// decision evidence is malformed or version-mismatched; fail loud, the
    /// guards stay (fail-closed to *retained* is never silent).
    ImageVerdictsInvalid { found: String },
}

impl CodecError {
    /// The diagnostic code the fail-closed paths surface (claims registry +
    /// error-registry appendix).
    pub fn code(&self) -> u32 {
        match self {
            CodecError::SchemaVersion { .. } => 6400,
            CodecError::ImageVerdictsInvalid { .. } => 6415,
            _ => 6401,
        }
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Serialize a complete obligation set to JSON bytes (fixed key order, Q11).
/// Enforces the 16 MiB cap on the write side too (NFR-5), and the P1.2
/// per-word IR cap of 8 KiB (E6401-class fail-closed).
pub fn encode_obl(set: &OblSet) -> Result<Vec<u8>, CodecError> {
    // PLAN-VERIFY-3 §6.1 (P1.2): per-word IR ≤ 8 KiB, else fail closed. A
    // word past the bound is an artifact the statement pipeline cannot carry.
    for w in &set.facts.words {
        if w.ir.len() > WORD_IR_MAX_BYTES {
            return Err(CodecError::WordIrTooLarge {
                word: w.name.clone(),
                size: w.ir.len(),
            });
        }
    }
    // Single validated constructor (P1.2 debt item): the identity arithmetic
    // is checked before anything is written — an underlying record whose id
    // or id_hash drifted from its canonical form is malformed, never encoded.
    set.validate().map_err(|_| CodecError::Malformed)?;
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
    out.extend_from_slice(b",\"stmt\":");
    write_str(out, &set.stmt);
    out.extend_from_slice(b",\"module\":");
    write_str(out, &set.module);
    out.extend_from_slice(b",\"target\":");
    write_str(out, &set.target);
    out.extend_from_slice(b",\"platform\":");
    write_str(out, &set.platform);
    out.extend_from_slice(b",\"model_semantics\":");
    write_str(out, &set.model_semantics);
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
        out.extend_from_slice(b",\"blocks\":");
        push_i64_json(out, w.blocks as i64);
        out.extend_from_slice(b",\"ir\":");
        write_str(out, &w.ir);
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
    // Slice P6: named contract-predicate facts (Q7). Always emitted — fixed
    // schema (`tyu.obl/v2` grew a member; readers skip unknown keys).
    out.extend_from_slice(b"],\"predicates\":[");
    for (i, p) in facts.predicates.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"name\":");
        write_str(out, &p.name);
        out.extend_from_slice(b",\"ir\":[");
        for (j, line) in p.ir.iter().enumerate() {
            if j != 0 {
                out.push(b',');
            }
            write_str(out, line);
        }
        out.extend_from_slice(b"],\"ir_hash\":");
        write_str(out, &p.ir_hash);
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
    out.extend_from_slice(b"}},\"intent\":{\"label\":");
    write_str(out, &o.intent.label);
    out.extend_from_slice(b",\"subject\":");
    write_str(out, &o.intent.subject);
    out.extend_from_slice(b",\"authored\":");
    out.extend_from_slice(if o.intent.authored { b"true" } else { b"false" });
    out.extend_from_slice(b"},\"formula\":");
    write_formula(out, &o.formula);
    out.extend_from_slice(b",\"assumptions\":[");
    for (i, a) in o.assumptions.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        write_assumption_edge(out, a);
    }
    out.extend_from_slice(b"],\"cycles\":[");
    for (i, c) in o.cycles.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"index\":");
        write_i64(out, c.index as i64);
        out.extend_from_slice(b",\"blocks\":[");
        for (j, b) in c.blocks.iter().enumerate() {
            if j != 0 {
                out.push(b',');
            }
            write_str(out, b);
        }
        out.extend_from_slice(b"]}");
    }
    out.extend_from_slice(b"],\"provenance\":");
    write_str(out, o.provenance.as_str());
    out.push(b'}');
}

/// §Q7 rule 1 (§6.1): the `assumptions` member is the dependency-edge list —
/// a bare `"runtime-check"` string (the emitted check IS the discharge) or an
/// `{"obligation": …, "module": …}` reference to the callee's obligation.
fn write_assumption_edge(out: &mut Vec<u8>, e: &crate::model::AssumptionEdge) {
    match e {
        crate::model::AssumptionEdge::Obligation { id, module } => {
            out.extend_from_slice(b"{\"obligation\":");
            write_str(out, id);
            out.extend_from_slice(b",\"module\":");
            write_str(out, module);
            out.push(b'}');
        }
        crate::model::AssumptionEdge::RuntimeCheck => {
            write_str(out, "runtime-check");
        }
    }
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
        Formula::PredicateHolds { pred, args } => {
            out.extend_from_slice(b"{\"op\":\"PredicateHolds\",\"predicate\":{");
            out.extend_from_slice(b"\"module\":");
            write_str(out, &pred.module);
            out.extend_from_slice(b",\"name\":");
            write_str(out, &pred.name);
            out.extend_from_slice(b",\"ir\":[");
            for (i, line) in pred.ir.iter().enumerate() {
                if i != 0 {
                    out.push(b',');
                }
                write_str(out, line);
            }
            out.extend_from_slice(b"],\"ir_hash\":");
            write_str(out, &pred.ir_hash);
            out.extend_from_slice(b"},\"args\":[");
            for (i, a) in args.iter().enumerate() {
                if i != 0 {
                    out.push(b',');
                }
                write_oel(out, a);
            }
            out.extend_from_slice(b"]}");
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
        let mut stmt: Option<String> = None;
        let mut module: Option<String> = None;
        let mut target: Option<String> = None;
        let mut platform: Option<String> = None;
        let mut model_semantics: Option<String> = None;
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
                "stmt" => stmt = Some(self.parse_string()?),
                "module" => module = Some(self.parse_string()?),
                "target" => target = Some(self.parse_string()?),
                "platform" => platform = Some(self.parse_string()?),
                "model_semantics" => model_semantics = Some(self.parse_string()?),
                "abi_contract_version" => abi_contract_version = Some(self.parse_u64()? as u32),
                "facts" => facts = Some(self.parse_facts()?),
                "obligations" => obligations = Some(self.parse_obligations()?),
                // Unknown keys are skipped — additive schema growth (owner doc §6).
                _ => self.skip_value()?,
            }
        }
        let (
            Some(schema),
            Some(semantics),
            Some(module),
            Some(abi_contract_version),
            Some(facts),
            Some(obligations),
        ) = (
            schema,
            semantics,
            module,
            abi_contract_version,
            facts,
            obligations,
        )
        else {
            return self.err();
        };
        Ok(OblSet {
            schema,
            semantics,
            // `stmt`/`target`/`platform`/`model_semantics` are P1.2 additions;
            // a pre-P1.2 (v1) artifact fails E6400 before reaching here, so an
            // in-band default is only a reader-side courtesy for malformed-but-
            // schema-stamped documents. Defaults keep the reader total.
            stmt: stmt.unwrap_or_else(|| crate::stmt::STMT_SCHEMA.to_string()),
            module,
            target: target.unwrap_or_default(),
            platform: platform.unwrap_or_default(),
            model_semantics: model_semantics
                .unwrap_or_else(|| crate::model::MODEL_UNMODELED.to_string()),
            abi_contract_version,
            facts,
            obligations,
        })
    }

    fn parse_facts(&mut self) -> Result<Facts, CodecError> {
        self.expect(b'{')?;
        let mut words: Option<Vec<WordFact>> = None;
        let mut subtypes: Option<Vec<SubtypeFact>> = None;
        let mut predicates: Option<Vec<PredicateFact>> = None;
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
                "predicates" => predicates = Some(self.parse_predicate_facts()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(words), Some(subtypes)) = (words, subtypes) else {
            return self.err();
        };
        // `predicates` is additive (slice P6); a pre-P6 artifact has none.
        Ok(Facts {
            words,
            subtypes,
            predicates: predicates.unwrap_or_default(),
        })
    }

    fn parse_predicate_facts(&mut self) -> Result<Vec<PredicateFact>, CodecError> {
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
            let mut ir: Option<Vec<String>> = None;
            let mut ir_hash: Option<String> = None;
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
                    "ir" => ir = Some(self.parse_string_array()?),
                    "ir_hash" => ir_hash = Some(self.parse_string()?),
                    _ => self.skip_value()?,
                }
            }
            let (Some(name), Some(ir), Some(ir_hash)) = (name, ir, ir_hash) else {
                return self.err();
            };
            out.push(PredicateFact { name, ir, ir_hash });
        }
        Ok(out)
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
            let mut blocks: Option<u32> = None;
            let mut ir: Option<String> = None;
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
                    "blocks" => blocks = Some(self.parse_u64()? as u32),
                    "ir" => ir = Some(self.parse_string()?),
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
                // `blocks`/`ir` are P1.2 additions — default on absent.
                ir: ir.unwrap_or_default(),
                blocks: blocks.unwrap_or(0),
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
        let mut intent: Option<crate::model::Intent> = None;
        let mut formula: Option<Formula> = None;
        let mut assumptions: Option<Vec<crate::model::AssumptionEdge>> = None;
        let mut cycles: Option<Vec<crate::model::Cycle>> = None;
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
                "intent" => intent = Some(self.parse_intent()?),
                "formula" => formula = Some(self.parse_formula()?),
                "assumptions" => assumptions = Some(self.parse_assumptions()?),
                "cycles" => cycles = Some(self.parse_cycles()?),
                "provenance" => {
                    let p = self.parse_string()?;
                    provenance = Provenance::from_str(&p);
                }
                // Unknown keys (a future additive growth, or a legacy member
                // like `assumption_edges`) are skipped — the v2 `assumptions`
                // member is the single dependency record.
                _ => self.skip_value()?,
            }
        }
        let (
            Some(id),
            Some(id_hash),
            Some(kind),
            Some(site),
            Some(formula),
            Some(assumptions),
            Some(provenance),
        ) = (id, id_hash, kind, site, formula, assumptions, provenance)
        else {
            return self.err();
        };
        Ok(Obligation {
            id,
            id_hash,
            kind,
            site,
            // `intent`/`assumptions`/`cycles` are P1.2 additions: intent
            // defaults to the synthesized form when the member is absent
            // (reader-side totality); edges and cycles default empty.
            intent: intent.unwrap_or_else(|| crate::model::default_intent(kind, "")),
            formula,
            assumptions,
            cycles: cycles.unwrap_or_default(),
            provenance,
        })
    }

    fn parse_intent(&mut self) -> Result<crate::model::Intent, CodecError> {
        self.expect(b'{')?;
        let mut label: Option<String> = None;
        let mut subject: Option<String> = None;
        let mut authored: Option<bool> = None;
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
                "label" => label = Some(self.parse_string()?),
                "subject" => subject = Some(self.parse_string()?),
                "authored" => authored = Some(self.parse_bool()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(label), Some(subject), Some(authored)) = (label, subject, authored) else {
            return self.err();
        };
        Ok(crate::model::Intent {
            label,
            subject,
            authored,
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
        Ok(Site {
            word,
            occurrence,
            span,
        })
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
        let mut pred: Option<PredicateRef> = None;
        let mut args: Option<Vec<Oel>> = None;
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
                "predicate" => pred = Some(self.parse_predicate_ref()?),
                "args" => args = Some(self.parse_oel_array()?),
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
            "PredicateHolds" => {
                let (Some(pred), Some(args)) = (pred, args) else {
                    return self.err();
                };
                Ok(Formula::PredicateHolds { pred, args })
            }
            _ => self.err(),
        }
    }

    fn parse_predicate_ref(&mut self) -> Result<PredicateRef, CodecError> {
        self.expect(b'{')?;
        let mut module: Option<String> = None;
        let mut name: Option<String> = None;
        let mut ir: Option<Vec<String>> = None;
        let mut ir_hash: Option<String> = None;
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
                "module" => module = Some(self.parse_string()?),
                "name" => name = Some(self.parse_string()?),
                "ir" => ir = Some(self.parse_string_array()?),
                "ir_hash" => ir_hash = Some(self.parse_string()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(module), Some(name), Some(ir), Some(ir_hash)) = (module, name, ir, ir_hash)
        else {
            return self.err();
        };
        Ok(PredicateRef {
            module,
            name,
            ir,
            ir_hash,
        })
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

    /// `[oel, oel, …]` — the `args` list of a `PredicateHolds` formula.
    fn parse_oel_array(&mut self) -> Result<Vec<Oel>, CodecError> {
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
            out.push(self.parse_oel()?);
        }
        Ok(out)
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

    /// §Q7 rule 1: parse the `assumptions` member — the dependency-edge list.
    /// An element is either the bare string `"runtime-check"` or an object
    /// `{"obligation": …, "module": …}` (the obligation form).
    fn parse_assumptions(&mut self) -> Result<Vec<crate::model::AssumptionEdge>, CodecError> {
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
            match self.peek() {
                Some(b'"') => {
                    let s = self.parse_string()?;
                    match s.as_str() {
                        "runtime-check" => out.push(crate::model::AssumptionEdge::RuntimeCheck),
                        _ => return self.err(),
                    }
                }
                Some(b'{') => {
                    self.expect(b'{')?;
                    let mut obligation: Option<String> = None;
                    let mut module: Option<String> = None;
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
                            "obligation" => obligation = Some(self.parse_string()?),
                            "module" => module = Some(self.parse_string()?),
                            _ => self.skip_value()?,
                        }
                    }
                    let (Some(obligation), Some(module)) = (obligation, module) else {
                        return self.err();
                    };
                    out.push(crate::model::AssumptionEdge::Obligation {
                        id: obligation,
                        module,
                    });
                }
                _ => return self.err(),
            }
        }
        Ok(out)
    }

    /// Parse the `cycles` member — the word's nontrivial SCC list (§Q9).
    fn parse_cycles(&mut self) -> Result<Vec<crate::model::Cycle>, CodecError> {
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
            let mut index: Option<u32> = None;
            let mut blocks: Option<Vec<String>> = None;
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
                    "index" => index = Some(self.parse_u64()? as u32),
                    "blocks" => blocks = Some(self.parse_string_array()?),
                    _ => self.skip_value()?,
                }
            }
            let (Some(index), Some(blocks)) = (index, blocks) else {
                return self.err();
            };
            out.push(crate::model::Cycle { index, blocks });
        }
        Ok(out)
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
            b'{' => loop {
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
            },
            b'[' => loop {
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
            },
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

// ---------------------------------------------------------------------------
// Slice P7: image-verdicts record (Q5/FR-11; E6415)
// ---------------------------------------------------------------------------

/// The image-level guard-elision decision record (`.tyu-verify/
/// image-verdicts.json`): the durable, schema-validated evidence behind the
/// report's `contexts.stack.guards` — what the report will say is exactly
/// what this record says (FR-16 spirit; a malformed record is E6415,
/// fail-loud, and the guards stay).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageVerdicts {
    /// `true` when the image's `stack-budget(main)` obligation discharged and
    /// pass 2 forwarded `--elide-ds-guards` (guards omitted).
    pub elided: bool,
    /// The main-context accounting the decision was made from (reported
    /// `high`/`top`/budget/verdict), so the record explains itself.
    pub main: crate::report::MainContextAccounting,
}

/// Serialize the two-pass elision decision. Fixed key order (Q11).
pub fn encode_image_verdicts(v: &ImageVerdicts) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(b"{\"schema\":");
    write_str(&mut out, crate::report::IMAGE_VERDICTS_SCHEMA);
    out.extend_from_slice(b",\"semantics\":");
    write_str(&mut out, crate::semantics::SEMANTICS_VERSION);
    out.extend_from_slice(b",\"guards\":");
    write_str(&mut out, if v.elided { "elided" } else { "retained" });
    out.extend_from_slice(b",\"main\":{\"high\":");
    write_i64(&mut out, v.main.high as i64);
    out.extend_from_slice(b",\"top\":");
    out.extend_from_slice(if v.main.top { b"true" } else { b"false" });
    out.extend_from_slice(b",\"budget\":");
    write_i64(&mut out, v.main.budget as i64);
    out.extend_from_slice(b",\"verdict\":");
    write_str(&mut out, &v.main.verdict);
    out.extend_from_slice(b"}}");
    if out.len() > OBL_ARTIFACT_MAX_BYTES {
        return Err(CodecError::TooLarge { size: out.len() });
    }
    Ok(out)
}

/// Read + validate the image-verdicts record. A schema/semantics mismatch or
/// a malformed shape is `ImageVerdictsInvalid` (E6415) — fail-loud: the
/// guards are never silently treated as retained off a corrupt record.
pub fn read_image_verdicts(bytes: &[u8]) -> Result<ImageVerdicts, CodecError> {
    let mut r = Reader { b: bytes, i: 0 };
    r.skip_ws();
    if r.bump()? != b'{' {
        return Err(CodecError::ImageVerdictsInvalid {
            found: String::new(),
        });
    }
    let mut schema: Option<String> = None;
    let mut semantics: Option<String> = None;
    let mut guards: Option<String> = None;
    let mut high: u32 = 0;
    let mut top = false;
    let mut budget: u32 = 0;
    let mut verdict: String = String::new();
    loop {
        r.skip_ws();
        match r.peek() {
            Some(b'}') => {
                let _ = r.bump();
                break;
            }
            Some(b',') => {
                r.i += 1;
            }
            _ => {}
        }
        r.skip_ws();
        let key = r.parse_string()?;
        r.expect(b':')?;
        match key.as_str() {
            "schema" => schema = Some(r.parse_string()?),
            "semantics" => semantics = Some(r.parse_string()?),
            "guards" => guards = Some(r.parse_string()?),
            "main" => {
                r.expect(b'{')?;
                loop {
                    r.skip_ws();
                    match r.peek() {
                        Some(b'}') => {
                            let _ = r.bump();
                            break;
                        }
                        Some(b',') => {
                            r.i += 1;
                        }
                        _ => {}
                    }
                    r.skip_ws();
                    let k = r.parse_string()?;
                    r.expect(b':')?;
                    match k.as_str() {
                        "high" => high = r.parse_u64()? as u32,
                        "top" => top = r.parse_bool()?,
                        "budget" => budget = r.parse_u64()? as u32,
                        "verdict" => verdict = r.parse_string()?,
                        _ => r.skip_value()?,
                    }
                }
            }
            _ => r.skip_value()?,
        }
    }
    let found = schema.unwrap_or_default();
    if found != crate::report::IMAGE_VERDICTS_SCHEMA {
        return Err(CodecError::ImageVerdictsInvalid { found });
    }
    let sem = semantics.unwrap_or_default();
    if sem != crate::semantics::SEMANTICS_VERSION {
        return Err(CodecError::ImageVerdictsInvalid { found: sem });
    }
    let elided = match guards.as_deref() {
        Some("elided") => true,
        Some("retained") => false,
        _ => {
            return Err(CodecError::ImageVerdictsInvalid {
                found: guards.unwrap_or_default(),
            })
        }
    };
    Ok(ImageVerdicts {
        elided,
        main: crate::report::MainContextAccounting {
            high,
            top,
            budget,
            verdict,
        },
    })
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
    // P6.2: the proof-pipeline section (additive member — the report is
    // produced by the single in-tree writer, so the fixed key order stays).
    out.extend_from_slice(b",\"proof\":{\"tool\":");
    write_str(&mut out, &r.proof.tool);
    out.extend_from_slice(b",\"harvest\":");
    write_str(&mut out, &r.proof.harvest);
    out.extend_from_slice(b",\"gen_digest\":");
    write_str(&mut out, &r.proof.gen_digest);
    out.extend_from_slice(b",\"vendor_digest\":");
    write_str(&mut out, &r.proof.vendor_digest);
    out.extend_from_slice(b",\"statements\":[");
    for (i, s) in r.proof.statements.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"module\":");
        write_str(&mut out, &s.module);
        out.extend_from_slice(b",\"rendered\":");
        write_i64(&mut out, s.rendered as i64);
        out.extend_from_slice(b",\"proven\":");
        write_i64(&mut out, s.proven as i64);
        out.extend_from_slice(b",\"omitted\":");
        write_i64(&mut out, s.omitted as i64);
        out.extend_from_slice(b",\"unproven\":");
        write_i64(&mut out, s.unproven as i64);
        out.extend_from_slice(b",\"candidates\":");
        write_i64(&mut out, s.candidates as i64);
        out.push(b'}');
    }
    out.extend_from_slice(b"]}");
    // P8.2: the per-image assumption-closure status (§Q7 rule 2) — the T-CL
    // walker's outcome. Additive report member (the in-tree writer owns the
    // fixed key order).
    out.extend_from_slice(b",\"closure\":{\"well_closed\":");
    out.extend_from_slice(if r.closure.well_closed {
        b"true"
    } else {
        b"false"
    });
    out.extend_from_slice(b",\"checked\":");
    write_i64(&mut out, r.closure.checked as i64);
    out.extend_from_slice(b",\"unresolved\":");
    write_i64(&mut out, r.closure.unresolved as i64);
    out.extend_from_slice(b"}");
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
        if let Some(reason) = &o.reason {
            out.extend_from_slice(b",\"reason\":");
            write_str(&mut out, reason);
        }
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
    out.extend_from_slice(b"],\"provably_failing\":[");
    for (i, p) in r.provably_failing.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"id\":");
        write_str(&mut out, &p.id);
        out.extend_from_slice(b",\"kind\":");
        write_str(&mut out, &p.kind);
        out.extend_from_slice(b",\"module\":");
        write_str(&mut out, &p.module);
        out.extend_from_slice(b",\"word\":");
        write_str(&mut out, &p.word);
        out.extend_from_slice(b",\"site\":");
        write_str(&mut out, &p.site);
        out.extend_from_slice(b",\"line\":");
        write_i64(&mut out, p.line as i64);
        out.extend_from_slice(b",\"note\":");
        write_str(&mut out, &p.note);
        out.push(b'}');
    }
    out.extend_from_slice(b"],\"retained\":[");
    for (i, r_ret) in r.retained.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"id\":");
        write_str(&mut out, &r_ret.id);
        out.extend_from_slice(b",\"reason\":");
        write_str(&mut out, &r_ret.reason);
        out.push(b'}');
    }
    out.extend_from_slice(b"],\"stale_verdicts\":");
    write_i64(&mut out, r.stale_verdicts as i64);
    out.extend_from_slice(b",\"verdict_sources\":{\"file\":");
    write_i64(&mut out, r.verdict_sources.file as i64);
    out.extend_from_slice(b",\"in_tree\":");
    write_i64(&mut out, r.verdict_sources.in_tree as i64);
    out.extend_from_slice(b"},\"emitted_checks\":{\"subtype_range\":");
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
    // P7.3: the shipped TCB boundary (§6.8).
    out.extend_from_slice(b"},\"tcb\":[");
    for (i, t) in r.tcb.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"id\":");
        write_str(&mut out, &t.id);
        out.extend_from_slice(b",\"what\":");
        write_str(&mut out, &t.what);
        out.extend_from_slice(b",\"status\":");
        write_str(&mut out, &t.status);
        out.push(b'}');
    }
    out.extend_from_slice(b"]}");
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
        // P12.1 (§7.3): the per-module `(target, model)` identity.
        out.extend_from_slice(b",\"target\":");
        write_str(out, &m.target);
        out.extend_from_slice(b",\"model\":");
        write_str(out, &m.model);
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
        // P7.3: the v2 trust×method×surface split.
        out.extend_from_slice(b"},\"trust\":{\"proof\":");
        write_i64(out, m.trust.proof as i64);
        out.extend_from_slice(b",\"checked\":");
        write_i64(out, m.trust.checked as i64);
        out.extend_from_slice(b",\"assumed\":");
        write_i64(out, m.trust.assumed as i64);
        out.extend_from_slice(b",\"open\":");
        write_i64(out, m.trust.open as i64);
        out.extend_from_slice(b"},\"methods\":{\"certificate\":");
        write_i64(out, m.methods.certificate as i64);
        out.extend_from_slice(b",\"rederive\":");
        write_i64(out, m.methods.rederive as i64);
        out.extend_from_slice(b",\"descriptor\":");
        write_i64(out, m.methods.descriptor as i64);
        out.extend_from_slice(b",\"stack_exact\":");
        write_i64(out, m.methods.stack_exact as i64);
        out.extend_from_slice(b",\"interval\":");
        write_i64(out, m.methods.interval as i64);
        out.extend_from_slice(b"},\"surfaces\":{\"source\":");
        write_i64(out, m.surfaces.source as i64);
        out.extend_from_slice(b",\"ir\":");
        write_i64(out, m.surfaces.ir as i64);
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
        );
        ctx.push_subtype_fact(b"Percent", 0, 100);
        // An emulated-aperture access (P3): OffsetLE with a const offset. The
        // aperture SIZE lives in the formula — there is no separate
        // trusted-facts member in v2 (PLAN-VERIFY-3 §6.1).
        ctx.begin_word(b"read");
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
        assert!(text.starts_with(
            "{\"schema\":\"tyu.obl/v2\",\"semantics\":\"tyu.ir-sem/1.0\",\"stmt\":\"tyu.stmt/1.0\",\"module\":\"Bank\",\"target\":\"\",\"platform\":\"\",\"model_semantics\":\"unmodeled\",\"abi_contract_version\":2,\"facts\""
        ));
        assert!(text
            .contains("\"obligations\":[{\"id\":\"Bank::clamp::subtype-range::0\",\"id_hash\":\""));
        assert!(text.contains("\"intent\":{\"label\":\"value must lie in subtype range\",\"subject\":\"\",\"authored\":false}"));
        assert!(text.contains("\"assumptions\":[]"));
        assert!(text.contains("\"cycles\":[]"));
        assert!(text.contains("\"provenance\":\"opaque\""));
        // Slice P6: the fixed schema grew `facts.predicates` (always
        // emitted, additive growth — readers skip unknown keys).
        assert!(text.contains("\"predicates\":[]"));
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
        assert_eq!(
            first, second,
            "encode -> decode -> encode must be byte-exact"
        );
    }

    #[test]
    fn read_rejects_wrong_schema() {
        let set = sample_set();
        let mut doc: String =
            String::from_utf8_lossy(&encode_obl(&set).expect("encode")).into_owned();
        doc = doc.replacen("tyu.obl/v2", "tyu.obl/v999", 1);
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
            read_obl(b"{\"schema\": \"tyu.obl/v2\""),
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
        for k in [
            Kind::SubtypeRange,
            Kind::ContractPre,
            Kind::ContractPost,
            Kind::StackBudget,
            Kind::MmioBounds,
        ] {
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
        let mut doc: String =
            String::from_utf8_lossy(&encode_obl(&set).expect("encode")).into_owned();
        // Insert a future-style key inside the top-level object.
        doc = doc.replace(",\"facts\"", ",\"future_extra\":{\"a\":1},\"facts\"");
        let parsed = read_obl(doc.as_bytes()).expect("additive keys are skipped");
        assert_eq!(parsed.module, "Bank");
        assert_eq!(parsed.obligations.len(), 2);
    }

    /// P1.2: the v2 identity fields round-trip and are schema-checked.
    #[test]
    fn v2_identity_fields_roundtrip() {
        let mut ctx = ExtractionCtx::new_with_identity(
            b"Bank",
            b"x86_64-unknown-none",
            b"x86_64-unknown-none",
            b"tyu.model/x86_64-unknown-none/1",
        );
        ctx.begin_word(b"clamp");
        ctx.record(
            Kind::SubtypeRange,
            Formula::InRange {
                value: Oel::Var {
                    name: "$top".to_string(),
                },
                lo: 0,
                hi: 100,
            },
            1,
            2,
            Provenance::Opaque,
        );
        let set = ctx.into_set();
        assert_eq!(set.stmt, crate::stmt::STMT_SCHEMA);
        let bytes = encode_obl(&set).expect("encode");
        assert!(String::from_utf8_lossy(&bytes).contains("\"target\":\"x86_64-unknown-none\""));
        let parsed = read_obl(&bytes).expect("decode");
        assert_eq!(parsed, set, "identity fields must round-trip");
        assert_eq!(parsed.target, "x86_64-unknown-none");
        assert_eq!(parsed.platform, "x86_64-unknown-none");
        assert_eq!(parsed.model_semantics, "tyu.model/x86_64-unknown-none/1");
        assert_eq!(parsed.stmt, crate::stmt::STMT_SCHEMA);
    }

    /// P1.2: intent (authored + synthesized), assumption edges, and cycles
    /// round-trip byte-exactly.
    #[test]
    fn v2_obligation_fields_roundtrip() {
        let mut ctx = ExtractionCtx::new(b"Bank");
        ctx.begin_word(b"withdraw");
        ctx.record_with_intent(
            Kind::ContractPre,
            Formula::PredicateHolds {
                pred: crate::model::PredicateRef {
                    module: "Math".to_string(),
                    name: "nonneg".to_string(),
                    ir: vec![
                        "block b0".to_string(),
                        "dup i64".to_string(),
                        "ret".to_string(),
                    ],
                    ir_hash: "11aa22bb".to_string(),
                },
                args: vec![Oel::Var {
                    name: "$top".to_string(),
                }],
            },
            4,
            5,
            Provenance::Opaque,
            crate::model::Intent {
                label: "withdraw never exceeds balance".to_string(),
                subject: "nonneg".to_string(),
                authored: true,
            },
        );
        ctx.push_assumption_edge(crate::model::AssumptionEdge::Obligation {
            id: "Math::clamp::contract-post::0".to_string(),
            module: "Math".to_string(),
        });
        ctx.push_assumption_edge(crate::model::AssumptionEdge::RuntimeCheck);
        ctx.attach_word_cycles(vec![crate::model::Cycle {
            index: 1,
            blocks: vec!["b1".to_string(), "b2".to_string()],
        }]);
        let set = ctx.into_set();
        assert!(set.obligations[0].intent.authored);
        assert_eq!(set.obligations[0].assumptions.len(), 2);
        assert_eq!(set.obligations[0].cycles.len(), 1);

        let bytes = encode_obl(&set).expect("encode");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("\"authored\":true"));
        assert!(text.contains("\"obligation\":\"Math::clamp::contract-post::0\""));
        assert!(text.contains(
            "\"assumptions\":[{\"obligation\":\"Math::clamp::contract-post::0\",\"module\":\"Math\"},\"runtime-check\"]"
        ));
        assert!(text.contains("\"cycles\":[{\"index\":1,\"blocks\":[\"b1\",\"b2\"]}]"));

        let parsed = read_obl(&bytes).expect("decode");
        assert_eq!(parsed, set, "P1.2 fields must round-trip byte-exactly");
    }

    /// P1.2: per-word IR beyond the 8 KiB cap fails closed at encode
    /// (E6401-class, `WordIrTooLarge`).
    #[test]
    fn word_ir_cap_fails_closed() {
        let mut ctx = ExtractionCtx::new(b"Bank");
        ctx.begin_word(b"big");
        ctx.push_word_fact_with_ir(
            b"big",
            ir::StackBound {
                net: 0,
                high: ir::High::Slots(1),
            },
            ir::EffectSet::empty(),
            &"x".repeat(WORD_IR_MAX_BYTES + 1),
            1,
        );
        let set = ctx.into_set();
        let err = encode_obl(&set).expect_err("oversized word IR must fail closed");
        assert!(matches!(err, CodecError::WordIrTooLarge { .. }));
        assert_eq!(err.code(), 6401);
    }
}
