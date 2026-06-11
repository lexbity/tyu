//! Host-side diagnostic decoder: resolves raw `DiagRecord` fields against a
//! module's `.lang.modinfo` to produce human-readable `Diagnostic` values.
//!
//! Gated behind `feature = "std"` because it depends on `std::collections::HashMap`
//! and `alloc::string::String` for the name table.
//!
//! ## abi_hash binding
//!
//! Every `ModinfoIndex` carries the `abi_hash` from the module's `.lang.modinfo`.
//! The caller supplies the expected `abi_hash` (from the runtime or ELF) and
//! `check_abi_hash` refuses decode on mismatch — a stale map produces a hard
//! `StaleMap` error, never a misattributed name.

use std::collections::HashMap;
use std::string::String;

use crate::claims;
use crate::DiagRecord;

/// Information about a single exported word, extracted from `.lang.modinfo`.
#[derive(Clone, Debug)]
pub struct WordInfo {
    /// Human-readable word name (UTF-8).
    pub name: String,
    /// Effect set bits (effect-context-model §2.1, abi-contract §4.1).
    pub effects: u16,
    /// Capability-kind bitset (§4.2 of abi-contract).
    pub requires_caps: u16,
    /// Declared data-stack high-water in slots; `0xFFFFFFFF` = ⊤.
    pub stack_bound: u32,
}

/// Index of all exported words in a loaded module, built from its
/// `.lang.modinfo` section bytes.
pub struct ModinfoIndex {
    /// The module's ABI compatibility hash (abi-contract §5).
    pub abi_hash: u64,
    words: HashMap<u64, WordInfo>,
}

impl ModinfoIndex {
    /// Build an index from raw `.lang.modinfo` section bytes (exports only).
    ///
    /// Returns `None` if the bytes are structurally invalid (bad magic,
    /// truncated, hash mismatch between export and meta entry).
    pub fn from_modinfo_bytes(data: &[u8]) -> Option<Self> {
        use lmod::modinfo;

        let hdr = modinfo::decode(data)?;
        let export_count = hdr.export_count;
        let abi_hash = hdr.abi_hash;
        let mut words = HashMap::with_capacity(export_count as usize);

        for i in 0..export_count {
            let exp = modinfo::read_export(data, i)?;

            // Read the word_meta entry that the export's value_off points to.
            let meta_off = exp.value_off as usize;
            let meta_end = meta_off + modinfo::WORD_META_SIZE as usize;
            if meta_end > data.len() {
                return None;
            }

            let meta_hash = u64::from_le_bytes(
                data[meta_off..meta_off + 8].try_into().ok()?,
            );
            // The hash in the meta entry must match the export's sym_hash
            // (abi-contract §3 invariant).
            if meta_hash != exp.sym_hash {
                return None;
            }

            let effects =
                u16::from_le_bytes(data[meta_off + 8..meta_off + 10].try_into().ok()?);
            let requires_caps =
                u16::from_le_bytes(data[meta_off + 10..meta_off + 12].try_into().ok()?);
            let stack_bound =
                u32::from_le_bytes(data[meta_off + 12..meta_off + 16].try_into().ok()?);

            words.insert(
                exp.sym_hash,
                WordInfo {
                    name: String::from_utf8_lossy(exp.name).into_owned(),
                    effects,
                    requires_caps,
                    stack_bound,
                },
            );
        }

        Some(ModinfoIndex { abi_hash, words })
    }

    /// Build an index from raw `.lang.debug` section bytes (all words).
    ///
    /// The debug section provides full coverage (including module-private
    /// words) but does NOT carry an `abi_hash`.  The caller should use
    /// `check_abi_hash` separately when a trusted expected hash is known.
    /// Returns `None` if the bytes are structurally invalid.
    pub fn from_debug_bytes(data: &[u8]) -> Option<Self> {
        use lmod::debugsec;

        let (count, _) = debugsec::decode_header(data)?;
        let abi_hash = 0; // debug section has no abi_hash
        let mut words = HashMap::with_capacity(count as usize);

        for i in 0..count {
            let entry = debugsec::read_entry(data, i)?;
            words.insert(
                entry.sym_hash,
                WordInfo {
                    name: String::from_utf8_lossy(entry.name).into_owned(),
                    effects: entry.effects,
                    requires_caps: 0, // not stored in debug section
                    stack_bound: entry.high,
                },
            );
        }

        Some(ModinfoIndex { abi_hash, words })
    }

    /// Verify that the index's `abi_hash` matches the expected value.
    ///
    /// Returns `Err(StaleMap)` on mismatch — a hard refusal, not a warning.
    pub fn check_abi_hash(&self, expected: u64) -> Result<(), DecodeError> {
        if self.abi_hash == expected {
            Ok(())
        } else {
            Err(DecodeError::StaleMap {
                expected,
                actual: self.abi_hash,
            })
        }
    }

    /// Look up a word by its full 64-bit FNV-1a hash.
    pub fn lookup(&self, hash: u64) -> Option<&WordInfo> {
        self.words.get(&hash)
    }

    /// Number of indexed words.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// Errors from the diagnostic decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The `.lang.modinfo` `abi_hash` does not match the expected value
    /// for the running image.  A stale map would produce wrong names.
    StaleMap {
        expected: u64,
        actual: u64,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::StaleMap { expected, actual } => {
                write!(
                    f,
                    "StaleMap: modinfo abi_hash {:#016x} does not match \
                     expected {:#016x} (image version mismatch)",
                    actual, expected,
                )
            }
        }
    }
}

/// A decoded, human-readable diagnostic from a raw `DiagRecord`.
///
/// Unlike the wire `DiagRecord`, this type resolves the `word_hash` to a
/// human-readable name (when the word is exported and modinfo is available)
/// and attaches a claim-text label to the trap code.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// Resolved word name, or `None` if the word is module-private and
    /// `.lang.debug` was not emitted.
    pub word_name: Option<String>,
    /// Numeric trap / claim code (from the effect-context / stack-bound
    /// band or a runtime trap code).
    pub trap_code: u16,
    /// Human-readable label for `trap_code` (e.g. `"STACK_OVERFLOW"`).
    pub claim_text: &'static str,
    /// Whether the diagnostic payload is from a language-emitted trap
    /// (`true`) or a hardware fault / un-annotated branch (`false`).
    pub valid: bool,
    /// Origin of this diagnostic (1 = in-guest B agent, 2 = gdbstub A escalation).
    pub origin: u8,
    /// Source line number from the compiler's `debug_trap_loc`.
    /// `0` means unknown.
    pub source_line: u32,
    /// Live data-stack depth at the trap point, in slots (measured).
    pub ds_depth: u32,
    /// Declared `high(word)` from the word's static bound, in slots.
    /// `0xFFFFFFFF` means unknown or ⊤.
    pub ds_declared: u32,
}

/// Resolve a raw wire `DiagRecord` against a `ModinfoIndex` to produce a
/// human-readable `Diagnostic`.
///
/// The caller must have already verified `index.check_abi_hash(...)` — this
/// function does not repeat that check.
pub fn resolve(record: &DiagRecord, index: &ModinfoIndex) -> Diagnostic {
    let word_name = index.lookup(record.word_hash).map(|w| w.name.clone());
    Diagnostic {
        word_name,
        trap_code: record.trap_code,
        claim_text: claims::claim_text(record.trap_code),
        valid: record.valid,
        origin: record.origin,
        source_line: record.source_line,
        ds_depth: record.ds_depth,
        ds_declared: record.ds_declared,
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref name) = self.word_name {
            write!(f, "trap in '{}'", name)?;
        } else {
            write!(f, "trap (unknown word)")?;
        }
        write!(f, ": {} ({})", self.claim_text, self.trap_code)?;
        if self.source_line > 0 {
            write!(f, " at line {}", self.source_line)?;
        }
        write!(f, " ds={}", self.ds_depth)?;
        if self.ds_declared != crate::DS_DECLARED_UNKNOWN {
            write!(f, "/{}", self.ds_declared)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagRecord, DS_DECLARED_UNKNOWN};
    use std::string::ToString;
    use std::vec::Vec;

    /// Helper: the expected abi_hash for test vectors.
    fn test_abi_hash() -> u64 {
        lmod::abi_hash::compute_abi_hash(8, 64, 2)
    }

    /// Build a minimal `.lang.modinfo` byte array for testing.
    fn make_modinfo(
        abi_hash: u64,
        exports: &[(&str, u16, u16, u32)],
    ) -> Vec<u8> {
        use lmod::modinfo;
        use lmod::hash::fnv1a_u64;

        let export_entries: Vec<modinfo::ExportEntry<'_>> = exports
            .iter()
            .map(|(name, effects, caps, bound)| modinfo::ExportEntry {
                sym_hash: fnv1a_u64(name.as_bytes()),
                name: name.as_bytes(),
                effects: *effects,
                requires_caps: *caps,
                stack_bound: *bound,
            })
            .collect();

        let mut buf = Vec::with_capacity(4096);
        buf.resize(4096, 0);
        let n = modinfo::encode_into(&mut buf, b"test", &export_entries, &[], abi_hash, 0, &[])
            .expect("encode modinfo");
        buf.truncate(n);
        buf
    }

    /// Build a test DiagRecord with the given word_hash.
    fn make_record(word_hash: u64, trap_code: u16) -> DiagRecord {
        DiagRecord {
            version: 1,
            origin: crate::origin::IN_GUEST,
            valid: true,
            trap_code,
            source_line: 42,
            word_hash,
            trap_pc: 0,
            ds_depth: 16,
            ds_declared: DS_DECLARED_UNKNOWN,
            slot_count: 0,
        }
    }

    // -------------------------------------------------------------------
    // ModinfoIndex construction
    // -------------------------------------------------------------------

    #[test]
    fn modinfo_index_single_export() {
        let data = make_modinfo(test_abi_hash(), &[("add", 0, 0, 42)]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.abi_hash, test_abi_hash());
        let hash = lmod::hash::fnv1a_u64(b"add");
        let info = idx.lookup(hash).unwrap();
        assert_eq!(info.name, "add");
        assert_eq!(info.stack_bound, 42);
    }

    #[test]
    fn modinfo_index_multiple_exports() {
        let data = make_modinfo(
            test_abi_hash(),
            &[
                ("add", 0, 0, 10),
                ("sub", 0, 0, 20),
                ("mul", 2, 1, 30),
            ],
        );
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        assert_eq!(idx.len(), 3);

        let add = idx.lookup(lmod::hash::fnv1a_u64(b"add")).unwrap();
        assert_eq!(add.name, "add");
        assert_eq!(add.stack_bound, 10);

        let sub = idx.lookup(lmod::hash::fnv1a_u64(b"sub")).unwrap();
        assert_eq!(sub.name, "sub");
        assert_eq!(sub.stack_bound, 20);

        let mul = idx.lookup(lmod::hash::fnv1a_u64(b"mul")).unwrap();
        assert_eq!(mul.name, "mul");
        assert_eq!(mul.effects, 2);
        assert_eq!(mul.requires_caps, 1);
        assert_eq!(mul.stack_bound, 30);
    }

    #[test]
    fn modinfo_index_empty() {
        let data = make_modinfo(test_abi_hash(), &[]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn modinfo_index_rejects_truncated() {
        assert!(ModinfoIndex::from_modinfo_bytes(b"").is_none());
        assert!(ModinfoIndex::from_modinfo_bytes(b"LMOD").is_none());
    }

    #[test]
    fn modinfo_index_rejects_bad_magic() {
        let mut data = make_modinfo(test_abi_hash(), &[]);
        data[0] = 0xFF;
        assert!(ModinfoIndex::from_modinfo_bytes(&data).is_none());
    }

    // -------------------------------------------------------------------
    // abi_hash check
    // -------------------------------------------------------------------

    #[test]
    fn abi_hash_matches() {
        let data = make_modinfo(test_abi_hash(), &[("f", 0, 0, 0)]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        assert!(idx.check_abi_hash(test_abi_hash()).is_ok());
    }

    #[test]
    fn abi_hash_mismatch_stale_map() {
        let data = make_modinfo(0xAAAAAAAAAAAAAAAA, &[("f", 0, 0, 0)]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        let err = idx.check_abi_hash(0xBBBBBBBBBBBBBBBB).unwrap_err();
        assert!(matches!(err, DecodeError::StaleMap { .. }));
        assert!(err.to_string().contains("StaleMap"));
    }

    // -------------------------------------------------------------------
    // resolve
    // -------------------------------------------------------------------

    #[test]
    fn resolve_named_word() {
        let data = make_modinfo(test_abi_hash(), &[("main", 0, 0, 128)]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        let hash = lmod::hash::fnv1a_u64(b"main");
        let record = make_record(hash, 10);

        let diag = resolve(&record, &idx);
        assert_eq!(diag.word_name.as_deref(), Some("main"));
        assert_eq!(diag.trap_code, 10);
        assert_eq!(diag.claim_text, "STACK_OVERFLOW");
        assert!(diag.valid);
        assert_eq!(diag.source_line, 42);
        assert_eq!(diag.ds_depth, 16);
        assert_eq!(diag.ds_declared, DS_DECLARED_UNKNOWN);
    }

    #[test]
    fn resolve_unknown_word_hash() {
        // word_hash that has no match in the modinfo
        let data = make_modinfo(test_abi_hash(), &[("main", 0, 0, 128)]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();

        let record = make_record(0xDEADBEEF, 5001);
        let diag = resolve(&record, &idx);
        assert!(diag.word_name.is_none());
        assert_eq!(diag.trap_code, 5001);
        assert_eq!(diag.claim_text, "E_SUSPEND_FORBIDDEN");
    }

    // -------------------------------------------------------------------
    // Display formatting
    // -------------------------------------------------------------------

    #[test]
    fn diagnostic_display_named() {
        let data = make_modinfo(test_abi_hash(), &[("main", 0, 0, 256)]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        let hash = lmod::hash::fnv1a_u64(b"main");
        let mut record = make_record(hash, 22);
        record.ds_declared = 256;

        let diag = resolve(&record, &idx);
        let text = diag.to_string();
        assert!(text.contains("main"));
        assert!(text.contains("ASSERT_FAIL"));
        assert!(text.contains("ds=16/256"));
    }

    #[test]
    fn diagnostic_display_unknown_word() {
        let data = make_modinfo(test_abi_hash(), &[]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        let record = make_record(0, 10);

        let diag = resolve(&record, &idx);
        let text = diag.to_string();
        assert!(text.contains("unknown word"));
        assert!(text.contains("STACK_OVERFLOW"));
    }

    #[test]
    fn diagnostic_display_line_zero() {
        let data = make_modinfo(test_abi_hash(), &[]);
        let idx = ModinfoIndex::from_modinfo_bytes(&data).unwrap();
        let mut record = make_record(0, 10);
        record.source_line = 0;

        let diag = resolve(&record, &idx);
        let text = diag.to_string();
        assert!(!text.contains("line"), "no line info should not emit 'line'");
    }
}
