//! The obligation model (static-verification.md §6.1, slice P2).
//!
//! An *obligation* is the proof object for one language-invariant runtime
//! check site: `(id, kind, site, formula, assumptions, meta)`. langc extracts
//! one record per site of classes C1–C7 regardless of discharge status; the
//! artifact interface — not any particular prover — is the product (Q1).
//!
//! Identity and stability (Q3, FR-9): the canonical id is
//! `"<module>::<word>::<kind>::<occurrence>"` where occurrence is the ordinal
//! among same-kind sites within the word in deterministic lowering order. Ids
//! NEVER derive from source spans. `id_hash` is FNV-1a-64 of the canonical id
//! (hex, 16 chars) — the cache key of record (Q3).
//!
//! Dependency direction (§5): this module knows only `ir` + its own model. A
//! `SubtypeInfo`-shaped fact enters via [`ExtractionCtx::push_subtype_fact`],
//! never via a `semantics` type.
//!
//! **No `serde`.** The prebuilt `alloc` rlib carries unwinding landing pads in
//! its serialization machinery (`String::from_utf8_lossy`,
//! `alloc::fmt::format`), which the hosted `#![no_std]` binaries'
//! `-nodefaultlibs` link cannot resolve; feature-unifying `serde` with the
//! std-flavored `serde` that `tyu` uses would put `std` in the `langc` binary
//! graph (duplicate `panic_impl`). The codecs therefore live in [`crate::codec`]
//! as hand-rolled, deterministic JSON — no serialization dependency at all.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use ir::{EffectSet, High, StackBound};

/// Schema identifier of `.obl.json` artifacts (`tyu.obl/v1`).
pub const OBL_SCHEMA: &str = "tyu.obl/v1";

/// Hard read-side cap for `.obl.json` artifacts (static-verification.md NFR-5;
/// enforced at encode and at read).
pub const OBL_ARTIFACT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Obligation classes (Q2). Closed, versioned enum: adding a member is a
/// schema change (`tyu.obl/v2`) because the `kind` string serializes into the
/// artifact and external tools match on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// A value of a subtype-typed place must lie in its declared range.
    SubtypeRange,
    /// A `needs` predicate must hold at a call site.
    ContractPre,
    /// An `ensures` predicate must hold at return.
    ContractPost,
    /// A context's data-stack peak must fit its `bounded-stack(N)` grant.
    StackBudget,
    /// An MMIO emulated-aperture access must stay within the aperture.
    MmioBounds,
}

impl Kind {
    /// Number of obligation classes; the extraction occurrence counters are
    /// indexed by this.
    pub const COUNT: usize = 5;

    pub const fn idx(self) -> usize {
        match self {
            Kind::SubtypeRange => 0,
            Kind::ContractPre => 1,
            Kind::ContractPost => 2,
            Kind::StackBudget => 3,
            Kind::MmioBounds => 4,
        }
    }

    /// The canonical kind string used in ids and the artifact.
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::SubtypeRange => "subtype-range",
            Kind::ContractPre => "contract-pre",
            Kind::ContractPost => "contract-post",
            Kind::StackBudget => "stack-budget",
            Kind::MmioBounds => "mmio-bounds",
        }
    }

    /// Parse the canonical kind string (artifact reader). The text `_`/`-`
    /// spelling MUST stay in sync with `as_str`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Kind> {
        Some(match s {
            "subtype-range" => Kind::SubtypeRange,
            "contract-pre" => Kind::ContractPre,
            "contract-post" => Kind::ContractPost,
            "stack-budget" => Kind::StackBudget,
            "mmio-bounds" => Kind::MmioBounds,
            _ => return None,
        })
    }
}

/// Formula provenance metadata (Q2/P2): whether the formula references values
/// the artifact can vouch for, or is a placeholder awaiting P5's interval
/// engine. `Opaque` formulas are dischargeable by no one — an honest
/// placeholder, never a silent proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    /// Formula references exact word inputs/outputs (`in.i` / `out.i`).
    Direct,
    /// Formula carries `$top` placeholders — no trusted value path (v1 casts).
    Opaque,
}

impl Provenance {
    pub const fn as_str(self) -> &'static str {
        match self {
            Provenance::Direct => "direct",
            Provenance::Opaque => "opaque",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Provenance> {
        match s {
            "direct" => Some(Provenance::Direct),
            "opaque" => Some(Provenance::Opaque),
            _ => None,
        }
    }
}

/// OEL (Obligation Expression Language) value expressions (Q2, §6.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Oel {
    /// A bound variable: word input `in.i`, word output `out.i`, or the
    /// opaque placeholder `$top`.
    Var { name: String },
    /// A narrowing cast — provenance metadata plus the from/to type names.
    /// The abstract transfer for this node is P5's concern (§7.2).
    Cast {
        from: String,
        to: String,
        arg: Box<Oel>,
    },
}

/// Obligation head predicates (Q2). `InRange(value, lo, hi)` is the head every
/// subtype-range site lowers to — the exact predicate `emit_subtype_range_trap`
/// implements at runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Formula {
    InRange { value: Oel, lo: i64, hi: i64 },
}

/// Trusted facts a formula may rely on (Q2 `assumptions`; T2 in Q14). P2 emits
/// no assumptions for the C1–C3 subtype sites (the range travels in the
/// formula itself); the enum is the closed set from which later slices add
/// descriptor facts (register ranges, stack geometry).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Assumption {
    /// The module declares a subtype with this range.
    SubtypeRange { name: String, lo: i64, hi: i64 },
}

/// Source span as *debug info only* — never part of an obligation's identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpanInfo {
    /// 1-based line; 0 means "no source location" (compiler-generated site).
    pub line: u32,
    pub col: u32,
}

/// Structural obligation site (Q2/Q3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Site {
    pub word: String,
    pub occurrence: u32,
    pub span: SpanInfo,
}

/// One obligation record (Q2, §6.1).
///
/// Field order is the schema order — the serializer writes fields in this
/// order, so the artifact's key order is fixed (Q11).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Obligation {
    pub id: String,
    pub id_hash: String,
    pub kind: Kind,
    pub site: Site,
    pub formula: Formula,
    pub assumptions: Vec<Assumption>,
    /// v1 transparency field (plan P2): distinguishes dischargeable-provenance
    /// formulas from `$top` placeholders. Appended so v1 writers remain
    /// readable by earlier consumers (readers skip unknown keys).
    pub provenance: Provenance,
}

/// A declared word's computed facts (Q7, §6.1 `facts.words`): the stack-bound
/// triple, effect set, and divergence freedom. Compiler-computed, never
/// hand-declared — the implementation of abi-contract §7's `def_word` record
/// for separate compilation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WordFact {
    pub name: String,
    /// `StackBound::net` — net data-stack slot delta of the word.
    pub net: i16,
    /// `StackBound::wire_u32`: peak slots, or `0xFFFF_FFFF` when `top`.
    pub high: u32,
    /// True when the word has no finite stack bound (`High::Top`).
    pub top: bool,
    /// Declared/computed effects, in bit order (SUSPEND, INTERRUPT, DIVERGE,
    /// MMIO, ALLOC).
    pub performs: Vec<String>,
    /// True when the word cannot diverge (no self-recursive call in its
    /// closure).
    pub diverge_free: bool,
}

/// A module subtype declaration fact (`facts.subtypes`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubtypeFact {
    pub name: String,
    pub lo: i64,
    pub hi: i64,
}

/// Module-level fact tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Facts {
    pub words: Vec<WordFact>,
    pub subtypes: Vec<SubtypeFact>,
}

/// The complete `.obl.json` document (§6.1). Top-level key order:
/// `schema`, `semantics`, `module`, `abi_contract_version`, `facts`,
/// `obligations` — fixed by the writer (Q11).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OblSet {
    pub schema: String,
    pub semantics: String,
    pub module: String,
    pub abi_contract_version: u32,
    pub facts: Facts,
    pub obligations: Vec<Obligation>,
}

/// Accumulation state threaded through lowering by `for_each_ir_word` /
/// `build_ir_word`: per-module fact tables plus per-(word, kind) occurrence
/// ordinals (Q3). Obligation ids are built here — one canonical place, one
/// hash computation.
pub struct ExtractionCtx {
    set: OblSet,
    /// Current word's canonical name (set by `begin_word`).
    word: Vec<u8>,
    /// Per-kind occurrence ordinals for the current word.
    counters: [u32; Kind::COUNT],
}

impl ExtractionCtx {
    pub fn new(module: &[u8]) -> Self {
        Self {
            set: OblSet {
                schema: OBL_SCHEMA.to_string(),
                semantics: crate::semantics::SEMANTICS_VERSION.to_string(),
                module: utf8_lossy(module),
                abi_contract_version: ir::contract::ABI_CONTRACT_VERSION as u32,
                facts: Facts {
                    words: Vec::new(),
                    subtypes: Vec::new(),
                },
                obligations: Vec::new(),
            },
            word: Vec::new(),
            counters: [0; Kind::COUNT],
        }
    }

    /// Start extracting for `word`: resets the per-word occurrence ordinals.
    /// Called once per declared word, before its lowering runs.
    pub fn begin_word(&mut self, word: &[u8]) {
        self.word = word.to_vec();
        self.counters = [0; Kind::COUNT];
    }

    /// Record one module subtype fact (iterated in declaration order).
    pub fn push_subtype_fact(&mut self, name: &[u8], lo: i64, hi: i64) {
        self.set.facts.subtypes.push(SubtypeFact {
            name: utf8_lossy(name),
            lo,
            hi,
        });
    }

    /// Record one declared word's computed facts, from its final IR word.
    pub fn push_word_fact(&mut self, name: &[u8], bound: StackBound, performs: EffectSet) {
        let top = bound.high.is_top();
        let high = match bound.high {
            High::Slots(n) => n,
            High::Top => u32::MAX,
        };
        let mut effs: Vec<String> = Vec::new();
        for (bit, label) in EFFECT_NAMES {
            if performs.contains(bit) {
                effs.push((*label).to_string());
            }
        }
        self.set.facts.words.push(WordFact {
            name: utf8_lossy(name),
            net: bound.net,
            high,
            top,
            performs: effs,
            diverge_free: !performs.contains(EffectSet::DIVERGE),
        });
    }

    /// Record one obligation at its extraction site. The occurrence ordinal is
    /// assigned here, deterministically (lowering order is deterministic).
    pub fn record(
        &mut self,
        kind: Kind,
        formula: Formula,
        line: u32,
        col: u32,
        provenance: Provenance,
    ) {
        let occurrence = self.counters[kind.idx()];
        self.counters[kind.idx()] = occurrence.wrapping_add(1);
        let id = canonical_id(&self.set.module, &self.word, kind, occurrence);
        self.set.obligations.push(Obligation {
            id_hash: format_hex(fnv1a64(id.as_bytes())),
            id,
            kind,
            site: Site {
                word: utf8_lossy(&self.word),
                occurrence,
                span: SpanInfo { line, col },
            },
            formula,
            assumptions: Vec::new(),
            provenance,
        });
    }

    /// Borrow the completed set (for writing the artifact).
    pub fn set(&self) -> &OblSet {
        &self.set
    }

    /// Consume the context, yielding the completed set.
    pub fn into_set(self) -> OblSet {
        self.set
    }
}

/// Effect names in wire-bit order (constant order, deterministic output).
const EFFECT_NAMES: [(u16, &str); 5] = [
    (EffectSet::SUSPEND, "SUSPEND"),
    (EffectSet::INTERRUPT, "INTERRUPT"),
    (EffectSet::DIVERGE, "DIVERGE"),
    (EffectSet::MMIO, "MMIO"),
    (EffectSet::ALLOC, "ALLOC"),
];

/// The canonical obligation id (Q3): `"<module>::<word>::<kind>::<occurrence>"`.
/// No `alloc::format!` (see [`push_u32_decimal`]).
pub fn canonical_id(module: &str, word: &[u8], kind: Kind, occurrence: u32) -> String {
    let mut out = alloc::string::String::new();
    out.push_str(module);
    out.push_str("::");
    out.push_str(core::str::from_utf8(word).unwrap_or("?"));
    out.push_str("::");
    out.push_str(kind.as_str());
    out.push_str("::");
    push_u32_decimal(&mut out, occurrence);
    out
}

/// FNV-1a 64-bit (the standard parameters; the ABI symbol hash uses the same
/// algorithm in `lmod::hash`, which this crate must not depend on — §5).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // offset basis
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // prime
    }
    h
}

/// Lowercase 16-hex-digit encoding of a u64 (the `id_hash` string form).
/// Hand-rolled: `alloc::format!` pulls `alloc::fmt::format_inner` from the
/// toolchain's prebuilt `alloc`, whose unwinding landing pad the hosted
/// `-nodefaultlibs` link cannot resolve.
pub fn format_hex(h: u64) -> String {
    let mut out = String::with_capacity(16);
    push_u64_hex(&mut out, h);
    out
}

/// Append `v` as lowercase hex to `out` (zero → "0"; no leading zeros).
pub fn push_u64_hex(out: &mut String, mut v: u64) {
    let mut buf = [0u8; 16];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        let d = (v & 0xF) as u8;
        buf[n] = match d {
            0..=9 => b'0' + d,
            _ => b'a' + (d - 10),
        };
        n += 1;
        v >>= 4;
    }
    buf[..n].reverse();
    out.push_str(core::str::from_utf8(&buf[..n]).unwrap_or(""));
}

/// Append `v` as decimal to `out` (zero → "0"; no leading zeros).
/// Same hand-rolled rationale as [`push_u64_hex`].
pub fn push_u32_decimal(out: &mut String, mut v: u32) {
    let mut buf = [0u8; 10];
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
    buf[..n].reverse();
    out.push_str(core::str::from_utf8(&buf[..n]).unwrap_or(""));
}

/// Lossy UTF-8 → String conversion without `alloc::string::String::from_utf8_lossy`.
///
/// The toolchain's prebuilt `alloc` rlib compiles `from_utf8_lossy` with an
/// unwinding landing pad, which the `hosted_rt` `-nodefaultlibs` link of the
/// `#![no_std]` host binaries cannot resolve (`_Unwind_Resume`). This module
/// runs inside exactly those binaries, so the conversion is done by hand over
/// `core::str` (unwinding-free): valid spans are copied, any invalid sequence
/// becomes U+FFFD. Identifiers this converts are ASCII in practice.
pub fn utf8_lossy(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match core::str::from_utf8(&bytes[i..]) {
            Ok(valid) => {
                out.push_str(valid);
                break;
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                out.push_str(core::str::from_utf8(&bytes[i..i + valid_up_to]).unwrap_or(""));
                match e.error_len() {
                    Some(len) => {
                        out.push('\u{FFFD}');
                        i += valid_up_to + len;
                    }
                    None => {
                        out.push('\u{FFFD}');
                        break;
                    }
                }
            }
        }
    }
    out
}