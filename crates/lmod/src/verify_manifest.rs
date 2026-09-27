//! The `verify_manifest` modinfo record (PLAN-VERIFY-3 §6.5, P11).
//!
//! A signed, in-module anchor: the build's verdict summary travels inside
//! the `.lang.modinfo` payload as a trailing, length-prefixed record so the
//! loader can validate and enforce it *before* any allocation.
//!
//! Wire layout (at the tail of the modinfo payload):
//!
//! ```text
//!   [ payload-size : u32 LE ][ tag : u32 LE = 0x31564D31 ("VM1") ]
//!   payload:
//!     u16 semantics_len, semantics bytes
//!     u16 stmt_len,      stmt bytes
//!     u16 target_len,    target bytes
//!     u16 model_len,     model bytes            ("unmodeled" allowed, §Q15)
//!     u8  policy                                0 open-ok · 1 no-open
//!                                               2 no-open-no-assumptions · 3 proven
//!     u8  certifier_class                       0 none · 1 port
//!     u16 certifier_name_len,      name bytes
//!     u16 certifier_recognition_len, recognition-id bytes
//!     u16 candidate_ratio                        basis 10000 (§Q10)
//!     u32 count                                  obligation entries (sorted by id)
//!     u32 counts[4]                              proof, checked, assumed, open
//!     -- obligations region (the canonical serialization; ids non-decreasing
//!        order — the writer sorts, the parser verifies):
//!     per entry:
//!       u16 id_len (≤ 512), id bytes
//!       u64 id_hash
//!       u8  status                              0 open · 1 discharged · 2 assumed
//!       u8  trust                               0 open · 1 assumed · 2 checked · 3 proof
//!       [32]u8 statement_hash
//!     [32]u8 digest                              SHA-256 over the obligations region
//! ```
//!
//! This crate is `#![no_std]` and has **no dependencies**: the record is
//! parsed by position over the caller's byte slice (same discipline as
//! `header.rs`); hashing is the caller's (the loader has SHA-256).

/// The record tag — `"VM1"` little-endian.
pub const VM_TAG: u32 = 0x31564d31;

// Policy encodings (§6.5 `policy`).
pub const VM_POLICY_OPEN_OK: u8 = 0;
pub const VM_POLICY_NO_OPEN: u8 = 1;
pub const VM_POLICY_NO_OPEN_NO_ASSUMPTIONS: u8 = 2;
pub const VM_POLICY_PROVEN: u8 = 3;

// Certifier class encodings.
pub const VM_CERTIFIER_NONE: u8 = 0;
pub const VM_CERTIFIER_PORT: u8 = 1;

// Status encodings (match `verifier::verdict::VerdictStatus` wire order).
pub const VM_STATUS_OPEN: u8 = 0;
pub const VM_STATUS_DISCHARGED: u8 = 1;
pub const VM_STATUS_ASSUMED: u8 = 2;

/// Does a module's *declared* policy satisfy a loader/deploy *requirement*?
///
/// `require` encoding mirrors `loader_core::platform::VerifyPolicy`:
/// `0` = Off (no requirement), `1` = RequireNoOpen, `2` = RequireProven.
/// Unknown requirements fail closed (never admit). This is the ONE policy
/// comparison both enforcers use — the loader's `validate_verify_manifest`
/// and the deploy gate's pairing check (the plan's §Q7 rule 3) — so the two
/// cannot drift. `lmod` is dependency-free: the comparison is byte-level.
pub fn satisfies(declared: u8, require: u8) -> bool {
    match require {
        0 => true,
        1 => declared >= VM_POLICY_NO_OPEN,
        2 => declared == VM_POLICY_PROVEN,
        _ => false,
    }
}

// Trust encodings (match `verifier::verdict::Trust` wire order).
pub const VM_TRUST_OPEN: u8 = 0;
pub const VM_TRUST_ASSUMED: u8 = 1;
pub const VM_TRUST_CHECKED: u8 = 2;
pub const VM_TRUST_PROOF: u8 = 3;

/// Caps (§6.5): id ≤ 512 B; the record ≤ 4 MiB (the inherited artifact cap).
pub const VM_MAX_ID_LEN: usize = 512;
pub const VM_MAX_RECORD: u32 = 4 * 1024 * 1024;
/// Upper bound on obligation entries (scan safety; not a semantic cap).
pub const VM_MAX_COUNT: u32 = 4096;

#[inline]
fn le_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

#[inline]
fn le_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[inline]
fn le_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

/// A parsed `verify_manifest` record, borrowing the caller's payload bytes.
#[derive(Clone, Copy, Debug)]
pub struct VerifyManifest<'a> {
    pub semantics: &'a [u8],
    pub stmt: &'a [u8],
    pub target: &'a [u8],
    pub model: &'a [u8],
    pub policy: u8,
    pub certifier_class: u8,
    pub certifier_name: &'a [u8],
    pub certifier_recognition: &'a [u8],
    pub candidate_ratio: u16,
    pub count: usize,
    pub counts: [u32; 4],
    /// Absolute offsets (into `data`) of the canonical obligations region —
    /// the bytes whose SHA-256 is the record's digest.
    pub obligations_start: usize,
    pub obligations_end: usize,
    pub digest: [u8; 32],
}

/// One obligation entry of the manifest.
#[derive(Clone, Copy, Debug)]
pub struct ObligationRef<'a> {
    /// The obligation id (≤ 512 B).
    pub id: &'a [u8],
    /// `fnv1a64` identity key.
    pub id_hash: u64,
    pub status: u8,
    pub trust: u8,
    /// SHA-256 statement binding (`tyu.stmt/1.0`).
    pub statement_hash: [u8; 32],
}

/// Parse an obligation entry positioned at `off` within `data`.
fn parse_obligation<'a>(data: &'a [u8], off: usize) -> Option<ObligationRef<'a>> {
    if off + 2 > data.len() {
        return None;
    }
    let id_len = le_u16(data, off) as usize;
    if id_len > VM_MAX_ID_LEN {
        return None;
    }
    let id_start = off + 2;
    let id_end = id_start + id_len;
    let rest = id_end + 8 + 1 + 1 + 32; // id_hash + status + trust + statement_hash
    if rest > data.len() {
        return None;
    }
    let id_hash = le_u64(data, id_end);
    let status = data[id_end + 8];
    let trust = data[id_end + 9];
    let mut statement_hash = [0u8; 32];
    statement_hash.copy_from_slice(&data[id_end + 10..id_end + 10 + 32]);
    Some(ObligationRef {
        id: &data[id_start..id_end],
        id_hash,
        status,
        trust,
        statement_hash,
    })
}

/// Step an obligation entry's start offset forward; `None` past the end.
fn next_obligation_offset(data: &[u8], off: usize) -> Option<usize> {
    if off + 2 > data.len() {
        return None;
    }
    let id_len = le_u16(data, off) as usize;
    let total = 2 + id_len + 8 + 1 + 1 + 32;
    if total > data.len().saturating_sub(off) {
        return None;
    }
    Some(off + total)
}

/// Scan and validate the `verify_manifest` record at the tail of `data`
/// (the modinfo payload). `Ok(None)` = no record present. Errors are the
/// 6500-class manifest-malformed code; values are **position-validated and
/// order-validated** (the obligations are walked once; ids must be
/// non-decreasing — the writer's canonical order).
pub fn scan_verify_manifest<'a>(data: &'a [u8]) -> Result<Option<VerifyManifest<'a>>, u32> {
    // [ payload ][ size:4 ][ tag:4 ] — tag is the FINAL 4 bytes.
    let total = data.len();
    if total < 8 {
        return Ok(None);
    }
    let tag = le_u32(data, total - 4);
    if tag != VM_TAG {
        return Ok(None);
    }
    let size = le_u32(data, total - 8) as usize;
    if size > VM_MAX_RECORD as usize {
        return Err(6500u32);
    }
    if size + 8 > total {
        return Err(6500u32);
    }
    let start = total - 8 - size;
    let payload = &data[start..start + size];
    let mut p = 0usize;
    let mut len = read_len(payload, &mut p)?;
    let semantics = read_bytes(payload, &mut p, len)?;
    len = read_len(payload, &mut p)?;
    let stmt = read_bytes(payload, &mut p, len)?;
    len = read_len(payload, &mut p)?;
    let target = read_bytes(payload, &mut p, len)?;
    len = read_len(payload, &mut p)?;
    let model = read_bytes(payload, &mut p, len)?;
    if p + 1 > payload.len() {
        return Err(6500u32);
    }
    let policy = payload[p];
    p += 1;
    if p + 1 > payload.len() {
        return Err(6500u32);
    }
    let certifier_class = payload[p];
    p += 1;
    len = read_len(payload, &mut p)?;
    let certifier_name = read_bytes(payload, &mut p, len)?;
    len = read_len(payload, &mut p)?;
    let certifier_recognition = read_bytes(payload, &mut p, len)?;
    if p + 2 > payload.len() {
        return Err(6500u32);
    }
    let candidate_ratio = le_u16(payload, p);
    p += 2;
    if p + 4 > payload.len() {
        return Err(6500u32);
    }
    let count = le_u32(payload, p) as usize;
    p += 4;
    if count > VM_MAX_COUNT as usize {
        return Err(6500u32);
    }
    if p + 16 > payload.len() {
        return Err(6500u32);
    }
    let counts = [
        le_u32(payload, p),
        le_u32(payload, p + 4),
        le_u32(payload, p + 8),
        le_u32(payload, p + 12),
    ];
    p += 16;
    // Enforce the closed sets up front (fail-closed, §6.3/§6.5).
    if policy > VM_POLICY_PROVEN || certifier_class > VM_CERTIFIER_PORT {
        return Err(6500u32);
    }
    // The obligations region: [p .. p(+entries) .. payload.end-32].
    let obligations_start = start + p;
    // Walk the entries (also validates the region bounds + sorted order).
    let mut off = p;
    let mut prev: Option<&[u8]> = None;
    for _ in 0..count {
        let entry = parse_obligation(payload, off).ok_or(6500u32)?;
        if entry.status > VM_STATUS_ASSUMED || entry.trust > VM_TRUST_PROOF {
            return Err(6500u32);
        }
        if let Some(prev_id) = prev {
            // Canonical order: ids must not strictly decrease.
            if entry.id < prev_id {
                return Err(6500u32);
            }
        }
        prev = Some(entry.id);
        off = next_obligation_offset(payload, off).ok_or(6500u32)?;
    }
    let obligations_end = start + off;
    // The digest rides immediately after the region.
    if off + 32 > payload.len() {
        return Err(6500u32);
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&payload[off..off + 32]);
    Ok(Some(VerifyManifest {
        semantics,
        stmt,
        target,
        model,
        policy,
        certifier_class,
        certifier_name,
        certifier_recognition,
        candidate_ratio,
        count,
        counts,
        obligations_start,
        obligations_end,
        digest,
    }))
}

/// Read the i-th obligation entry (0-based) of a scanned manifest.
pub fn obligation<'a>(
    vm: &VerifyManifest<'a>,
    data: &'a [u8],
    idx: usize,
) -> Option<ObligationRef<'a>> {
    if idx >= vm.count {
        return None;
    }
    // Offsets are absolute into `data`; walk from the region start.
    let mut off = vm.obligations_start;
    for _ in 0..idx {
        off = next_obligation_offset(data, off)?;
    }
    parse_obligation(data, off)
}

/// The index of the first obligation with `id` (binary search over the
/// sorted region); `None` if absent.
pub fn find_obligation<'a>(vm: &VerifyManifest<'a>, data: &'a [u8], id: &[u8]) -> Option<usize> {
    let (mut lo, mut hi) = (0usize, vm.count);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let e = obligation(vm, data, mid)?;
        match e.id.cmp(id) {
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Greater => hi = mid,
            core::cmp::Ordering::Equal => return Some(mid),
        }
    }
    None
}

/// A len-prefixed slice inside the payload; advances `p`.
fn read_len(payload: &[u8], p: &mut usize) -> Result<usize, u32> {
    if *p + 2 > payload.len() {
        return Err(6500u32);
    }
    let len = le_u16(payload, *p) as usize;
    *p += 2;
    Ok(len)
}

fn read_bytes<'a>(payload: &'a [u8], p: &mut usize, len: usize) -> Result<&'a [u8], u32> {
    if *p + len > payload.len() {
        return Err(6500u32);
    }
    let b = &payload[*p..*p + len];
    *p += len;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satisfies_matches_off_no_open_proven() {
        // Off (0): any declared policy satisfies.
        assert!(satisfies(0, 0));
        assert!(satisfies(3, 0));
        // RequireNoOpen (1): declared ≥ no-open.
        assert!(!satisfies(VM_POLICY_OPEN_OK, 1));
        assert!(satisfies(VM_POLICY_NO_OPEN, 1));
        assert!(satisfies(VM_POLICY_NO_OPEN_NO_ASSUMPTIONS, 1));
        assert!(satisfies(VM_POLICY_PROVEN, 1));
        // RequireProven (2): declared == proven.
        assert!(!satisfies(VM_POLICY_NO_OPEN, 2));
        assert!(!satisfies(VM_POLICY_NO_OPEN_NO_ASSUMPTIONS, 2));
        assert!(satisfies(VM_POLICY_PROVEN, 2));
        // Unknown requirements fail closed.
        assert!(!satisfies(VM_POLICY_PROVEN, 99));
    }

    #[test]
    fn satisfies_is_the_loader_deploy_shared_rule() {
        // The two enforcers' encodings agree: loader VerifyPolicy
        // Off=0 / RequireNoOpen=1 / RequireProven=2, and the deploy gate
        // maps its DeployVerifyPolicy onto the same 0/1/2.
        assert!(satisfies(VM_POLICY_NO_OPEN, 1));
        assert!(satisfies(VM_POLICY_PROVEN, 2));
    }
}
