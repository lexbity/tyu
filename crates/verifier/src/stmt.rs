//! Canonical statement encoding + `statement_hash` (PLAN-VERIFY-3 §6.2, P1).
//!
//! A *statement* is the hash-bound object a developer's proof binds to:
//! `{ context, formula }`, where `context` identifies the target- and
//! model-relative semantic surface and `formula` is the obligation's formula
//! object. The canonical form is the single interface between the obligation
//! extractor, the statement renderer (por), and the harvester — there is no
//! separate spec-language artifact (PLAN-VERIFY-3 §Q4 item 4).
//!
//! Canonical form (RFC 8785-style, per PLAN-VERIFY-3 §6.2):
//! - UTF-8 JSON, no whitespace;
//! - object keys in lexicographic byte order (the [`CanonObj`] writer owns the
//!   ordering — callers may construct fields in any order and get identical
//!   bytes);
//! - arrays in declared order.
//!
//! `statement_hash` is SHA-256 over the canonical bytes (FR-14: every
//! integrity digest introduced by this plan is SHA-256; fnv1a64 in this module
//! is forbidden by construction — `ci/guards.sh` gate G24 greps it out).
//!
//! The statement band rule (§Q4 item 3): within one toolchain semver band
//! (same tyu major.minor AND same `SEMANTICS_VERSION` AND same major of
//! `tyu.stmt`), statement hashes MUST be stable. Enforcement is the
//! statement-golden CI gate (`tooling-tests/tests/statement_goldens.rs`, P1.3).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::codec::{push_i64_json, push_str_json};
use crate::model::{Formula, Kind, Oel, PredicateRef};

/// Schema identifier of the canonical statement encoding (`tyu.stmt/1.0`).
pub const STMT_SCHEMA: &str = "tyu.stmt/1.0";

/// The statement context (§6.2): every field an obligation's claim is
/// relativized to. A verdict whose recorded context differs from the
/// consuming build's is stale — fail-closed to open (E6421 path at langc).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatementContext {
    /// `SEMANTICS_VERSION` of the producing toolchain.
    pub semantics: String,
    /// The statement-schema version (`tyu.stmt/1.0`).
    pub stmt_ver: String,
    /// Target triple the statement is proven for (§Q3 — semantics is a
    /// function of `(target, model_semantics)`, never a constant).
    pub target: String,
    /// Model-semantics identity (`tyu.model/<…>/<ver>`, or `"unmodeled"`).
    /// A statement is meaningless for a consuming build whose model differs.
    pub model_semantics: String,
    /// Defining module (artifact `module` field).
    pub module: String,
    /// The word the obligation's site lives in.
    pub word: String,
    /// Hash of the word's canonical op text (16 lowercase hex chars of
    /// SHA-256; §6.1 `facts.words[].ir_hash`). Binds the statement to the
    /// exact IR the obligation was extracted from.
    pub word_ir_hash: String,
    /// The obligation kind (closed enum, see [`Kind`]).
    pub kind: Kind,
    /// Site occurrence within the word (canonical id's trailing ordinal).
    pub occurrence: u32,
    /// Optional device refinement name (§Q13, P13). `None` = raw
    /// nondeterministic MMIO reads.
    pub refinement: Option<String>,
}

impl StatementContext {
    /// Build the context for an obligation record: relativity fields are
    /// caller-supplied, site/kind identity come from the record.
    pub fn for_obligation(
        module: &str,
        target: &str,
        model_semantics: &str,
        word_ir_hash: &str,
        o: &crate::model::Obligation,
    ) -> Self {
        Self {
            semantics: crate::semantics::SEMANTICS_VERSION.to_string(),
            stmt_ver: STMT_SCHEMA.to_string(),
            target: target.to_string(),
            model_semantics: model_semantics.to_string(),
            module: module.to_string(),
            word: o.site.word.clone(),
            word_ir_hash: word_ir_hash.to_string(),
            kind: o.kind,
            occurrence: o.site.occurrence,
            refinement: None,
        }
    }

    /// The canonical statement bytes `{"context":…,"formula":…}` — a total
    /// function of `(self, formula)`; two runs on equal inputs are
    /// byte-identical (FR-2).
    pub fn canonical_bytes(&self, formula: &Formula) -> Vec<u8> {
        let mut out = Vec::with_capacity(512);
        out.extend_from_slice(b"{\"context\":");
        push_context(&mut out, self);
        out.extend_from_slice(b",\"formula\":");
        push_formula(&mut out, formula);
        out.push(b'}');
        out
    }

    /// SHA-256 of the canonical statement bytes (the Sec. 6.2
    /// `statement_hash`).
    pub fn statement_hash(&self, formula: &Formula) -> [u8; 32] {
        sha256(&self.canonical_bytes(formula))
    }

    /// `statement_hash` as 64 lowercase hex characters.
    pub fn statement_hash_hex(&self, formula: &Formula) -> String {
        hex32(sha256(&self.canonical_bytes(formula)))
    }
}

// ---------------------------------------------------------------------------
// Canonical writer
// ---------------------------------------------------------------------------

/// A canonical JSON object: keys are emitted lexicographically regardless of
/// insertion order (RFC 8785-style key ordering; §6.2). Values are
/// pre-serialized bytes; nested objects are built recursively with this type.
struct CanonObj {
    keys: Vec<(&'static str, Vec<u8>)>,
}

impl CanonObj {
    fn new() -> Self {
        Self { keys: Vec::new() }
    }

    /// Insert one member. The writer sorts on finish — insertion order is
    /// irrelevant (the caller never owns the key order).
    fn member(&mut self, key: &'static str, value: Vec<u8>) {
        self.keys.push((key, value));
    }

    fn finish(self, out: &mut Vec<u8>) {
        let mut keys = self.keys;
        keys.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        out.push(b'{');
        for (i, (k, v)) in keys.iter().enumerate() {
            if i != 0 {
                out.push(b',');
            }
            push_str_json(out, k);
            out.push(b':');
            out.extend_from_slice(v);
        }
        out.push(b'}');
    }
}

fn v_str(s: &str) -> Vec<u8> {
    let mut v = Vec::new();
    push_str_json(&mut v, s);
    v
}

fn v_i64(n: i64) -> Vec<u8> {
    let mut v = Vec::new();
    push_i64_json(&mut v, n);
    v
}

fn v_null() -> Vec<u8> {
    b"null".to_vec()
}

fn v_array(items: Vec<Vec<u8>>) -> Vec<u8> {
    let mut v = Vec::with_capacity(64);
    v.push(b'[');
    for (i, item) in items.iter().enumerate() {
        if i != 0 {
            v.push(b',');
        }
        v.extend_from_slice(item);
    }
    v.push(b']');
    v
}

/// `{"context":…}` — context keys in lexicographic order.
fn push_context(out: &mut Vec<u8>, c: &StatementContext) {
    let mut o = CanonObj::new();
    o.member("kind", v_str(c.kind.as_str()));
    o.member("model_semantics", v_str(&c.model_semantics));
    o.member("module", v_str(&c.module));
    o.member("occurrence", v_i64(c.occurrence as i64));
    o.member(
        "refinement",
        match &c.refinement {
            Some(r) => v_str(r),
            None => v_null(),
        },
    );
    o.member("semantics", v_str(&c.semantics));
    o.member("stmt_ver", v_str(&c.stmt_ver));
    o.member("target", v_str(&c.target));
    o.member("word", v_str(&c.word));
    o.member("word_ir_hash", v_str(&c.word_ir_hash));
    o.finish(out);
}

/// Canonical formula object. `op` discriminates; every member's key is
/// lexicographically ordered.
fn push_formula(out: &mut Vec<u8>, f: &Formula) {
    let mut o = CanonObj::new();
    match f {
        Formula::InRange { value, lo, hi } => {
            o.member("hi", v_i64(*hi));
            o.member("lo", v_i64(*lo));
            o.member("op", v_str("InRange"));
            o.member("value", canon_oel(value));
        }
        Formula::OffsetLE { off, width, size } => {
            o.member(
                "off",
                match off {
                    Some(off) => v_i64(*off as i64),
                    None => v_null(),
                },
            );
            o.member("op", v_str("OffsetLE"));
            o.member("size", v_i64(*size as i64));
            o.member("width", v_i64(*width as i64));
        }
        Formula::PredicateHolds { pred, args } => {
            o.member("args", v_array(args.iter().map(canon_oel).collect()));
            o.member("op", v_str("PredicateHolds"));
            o.member("predicate", canon_predicate(pred));
        }
    }
    o.finish(out);
}

/// Canonical OEL value (`Var` / `Cast`).
fn canon_oel(o: &Oel) -> Vec<u8> {
    let mut obj = CanonObj::new();
    match o {
        Oel::Var { name } => {
            obj.member("name", v_str(name));
            obj.member("op", v_str("Var"));
        }
        Oel::Cast { from, to, arg } => {
            obj.member("arg", canon_oel(arg));
            obj.member("from", v_str(from));
            obj.member("op", v_str("Cast"));
            obj.member("to", v_str(to));
        }
    }
    let mut out = Vec::with_capacity(64);
    obj.finish(&mut out);
    out
}

/// Canonical predicate reference (module, name, transcluded IR + its hash).
fn canon_predicate(p: &PredicateRef) -> Vec<u8> {
    let mut obj = CanonObj::new();
    obj.member("ir", v_array(p.ir.iter().map(|l| v_str(l)).collect()));
    obj.member("ir_hash", v_str(&p.ir_hash));
    obj.member("module", v_str(&p.module));
    obj.member("name", v_str(&p.name));
    let mut out = Vec::with_capacity(128);
    obj.finish(&mut out);
    out
}

// ---------------------------------------------------------------------------
// SHA-256 (sha2 with default-features disabled — alloc-compatible; the same
// path loader-core uses) + hand-rolled hex (no alloc::format!, which the
// hosted -nodefaultlibs link cannot carry).
// ---------------------------------------------------------------------------

/// SHA-256 of `bytes` — the only message digest this module (and the P1
/// codec surface) may construct (FR-14). Guarded by `ci/guards.sh` G24.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut h = [0u8; 32];
    h.copy_from_slice(&out);
    h
}

/// 16 lowercase hex characters of SHA-256 — the truncated `word_ir_hash`
/// form (§6.1 `<sha256:16>`). FNV stays confined to `id_hash`; the word-IR
/// integrity digest is SHA-256 (FR-14).
pub fn sha256_hex16(bytes: &[u8]) -> String {
    let h = sha256(bytes);
    let mut out = String::with_capacity(16);
    let mut i = 0usize;
    while i < 8 {
        let d = (h[i] >> 4) & 0xF;
        out.push(HEX[d as usize] as char);
        out.push(HEX[(h[i] & 0xF) as usize] as char);
        i += 1;
    }
    out
}

/// 64 lowercase hex characters for a full SHA-256 digest.
pub fn hex32(h: [u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for &b in h.iter() {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xF) as usize] as char);
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

use sha2::Digest;
