//! The `tyu.spans/1` span side table (PLAN-WASM-2 B1, DEC-40): the custom
//! section that maps hardware-level wasm trap points to source spans, so a
//! backend-mode diagnostic can ALWAYS carry a span (FR-43 — "a backend-mode
//! diagnostic without a span is a defect").
//!
//! Two diagnostic paths exist on the wasm substrate:
//! 1. **Out-of-line checks** (`TrapIfFalse`, the checker's emitted subtype
//!    guards, region-ABI argument traps) call the `tyu.trap` import with
//!    `(code, span_start, span_len)` — the span travels at runtime, no side
//!    table needed.
//! 2. **Hardware traps** (an out-of-bounds `load`/`store`, division traps)
//!    never reach the host through an import; the engine reports the trapping
//!    instruction's code offset. The host trampoline consults THIS table to
//!    resolve that offset to a span.
//!
//! Format (version-first, fail-closed consumers — B2's trampoline MUST reject
//! unknown versions rather than guess):
//!
//! ```text
//! custom section "tyu.spans":
//!   uleb version        (= 1; consumers fail closed above it)
//!   uleb record_count
//!   per record:
//!     uleb code_offset  — byte offset of the trapping instruction, relative
//!                         to the start of the CODE section's payload (stable:
//!                         call operands are fixed-width, so patching never
//!                         shifts offsets)
//!     uleb span_start   — I-9 span encoding (UTF-8 byte offset into the
//!                         session/compilation buffer)
//!     uleb span_len
//! ```
//!
//! B1 emits one record per word-body `Load`/`Store` (the only word-level ops
//! that can trap at the hardware level — every other failure is an
//! out-of-line check). B2 extends coverage to the MMIO lowering as it lands.

use alloc::vec;
use alloc::vec::Vec;

/// The section name (custom section id 0, name payload below).
pub const SECTION_NAME: &[u8] = b"tyu.spans";

/// The format version this backend emits. Consumers fail closed above it.
pub const VERSION: u64 = 1;

/// One trap-point → span record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpanRecord {
    /// Byte offset of the trapping instruction, relative to the start of the
    /// CODE section payload.
    pub code_offset: u32,
    /// I-9 span encoding.
    pub span_start: u32,
    pub span_len: u32,
}

/// Encode the section payload (what lives INSIDE the named custom section —
/// the caller frames it with [`crate::encoder::section`] + [`crate::encoder::name`]).
pub fn encode(records: &[SpanRecord], out: &mut Vec<u8>) {
    crate::encoder::uleb(out, VERSION);
    crate::encoder::uleb(out, records.len() as u64);
    for r in records {
        crate::encoder::uleb(out, r.code_offset as u64);
        crate::encoder::uleb(out, r.span_start as u64);
        crate::encoder::uleb(out, r.span_len as u64);
    }
}

/// Encode the full custom section (id 0, named `tyu.spans/1`).
pub fn section(records: &[SpanRecord], out: &mut Vec<u8>) {
    crate::encoder::section(out, 0, |b| {
        crate::encoder::name(b, SECTION_NAME);
        encode(records, b);
    });
}

/// Decode a payload produced by [`encode`] — the fail-closed reference
/// consumer of the format (B2's trampoline mirrors this reading; the G54
/// harness parse-backs every emitted section through it). Unknown versions
/// fail CLOSED (an Err, never a guess at an unknown layout).
#[allow(dead_code)]
pub fn decode(payload: &[u8]) -> Result<Vec<SpanRecord>, ()> {
    let mut i = 0usize;
    let version = uleb_at(payload, &mut i).ok_or(())?;
    if version != VERSION {
        // Fail closed: a consumer that does not know the layout MUST NOT
        // attempt to parse it (DEC-40).
        return Err(());
    }
    let count = uleb_at(payload, &mut i).ok_or(())?;
    let mut out = vec![];
    for _ in 0..count {
        let code_offset = uleb_at(payload, &mut i).ok_or(())?;
        let span_start = uleb_at(payload, &mut i).ok_or(())?;
        let span_len = uleb_at(payload, &mut i).ok_or(())?;
        if code_offset > u32::MAX as u64 || span_start > u32::MAX as u64 || span_len > u32::MAX as u64
        {
            return Err(());
        }
        out.push(SpanRecord {
            code_offset: code_offset as u32,
            span_start: span_start as u32,
            span_len: span_len as u32,
        });
    }
    if i != payload.len() {
        // Trailing bytes: the producer's layout is fully known at this
        // version, so leftovers mean corruption — reject, never truncate.
        return Err(());
    }
    Ok(out)
}

#[allow(dead_code)]
fn uleb_at(payload: &[u8], i: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *payload.get(*i)?;
        *i += 1;
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let records = [
            SpanRecord { code_offset: 0, span_start: 0, span_len: 0 },
            SpanRecord { code_offset: 42, span_start: 120, span_len: 14 },
            SpanRecord {
                code_offset: u32::MAX,
                span_start: u32::MAX,
                span_len: u32::MAX,
            },
        ];
        let mut payload = Vec::new();
        encode(&records, &mut payload);
        assert_eq!(decode(&payload).unwrap(), records.to_vec());
    }

    #[test]
    fn empty_table_is_a_valid_version_header() {
        let mut payload = Vec::new();
        encode(&[], &mut payload);
        assert_eq!(payload, vec![0x01, 0x00]);
        assert!(decode(&payload).unwrap().is_empty());
    }

    #[test]
    fn unknown_version_fails_closed() {
        let mut payload = Vec::new();
        crate::encoder::uleb(&mut payload, VERSION + 1);
        crate::encoder::uleb(&mut payload, 0);
        assert!(decode(&payload).is_err());
    }

    #[test]
    fn truncated_payload_is_rejected() {
        let records = [SpanRecord { code_offset: 1, span_start: 2, span_len: 3 }];
        let mut payload = Vec::new();
        encode(&records, &mut payload);
        payload.pop(); // cut the last record field
        assert!(decode(&payload).is_err());
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut payload = Vec::new();
        encode(&[], &mut payload);
        payload.push(0xff); // not part of the v1 layout
        assert!(decode(&payload).is_err());
    }

    #[test]
    fn frames_as_a_named_custom_section() {
        let records = [SpanRecord { code_offset: 9, span_start: 8, span_len: 7 }];
        let mut out = Vec::new();
        section(&records, &mut out);
        // id 0, len, uleb(name len) + name, then the payload.
        assert_eq!(out[0], 0);
        let name_len = out[2] as usize;
        assert_eq!(&out[3..3 + name_len], SECTION_NAME);
        let payload = out[3 + name_len..].to_vec();
        assert_eq!(decode(&payload).unwrap(), records.to_vec());
    }
}
