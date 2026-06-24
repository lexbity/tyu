//! `.lang.debug` section — full-coverage word table.
//!
//! Unlike `.lang.modinfo` (exports only), this section lists **every** word
//! in a module with its full metadata, so module-private words' traps can
//! be resolved at diagnostic time.
//!
//! ## Wire format (little-endian)
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0      | 4    | `count` — number of word records (u32) |
//! | 4      | N    | Name table: `count` null-terminated strings, then 4-byte pad |
//! | 4+N    | M    | `count` word records, each 24 bytes |
//!
//! Each 24-byte record:
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0      | 8    | `sym_hash` — fnv1a_u64 of word name (u64) |
//! | 8      | 4    | `name_off` — byte offset into name table (u32) |
//! | 12     | 2    | `net` — stack effect net change (i16) |
//! | 14     | 4    | `high` — stack high-water in slots, 0xFFFFFFFF = ⊤ (u32) |
//! | 18     | 2    | `effects` — effect bitset (u16) |
//! | 20     | 4    | reserved, must be 0 |

use core::fmt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Size of each word record in the table, in bytes.
pub const DEBUG_RECORD_SIZE: u32 = 24;

/// Maximum number of word entries supported by the encoder.
pub const MAX_DEBUG_ENTRIES: usize = 4096;

// ---------------------------------------------------------------------------
// Entry types
// ---------------------------------------------------------------------------

/// A single word entry in the debug section, as passed to the encoder.
#[derive(Clone, Copy)]
pub struct DebugEntry<'a> {
    pub sym_hash: u64,
    /// Name bytes (must not be empty; should not contain NUL).
    pub name: &'a [u8],
    /// Stack effect net change.
    pub net: i16,
    /// Stack high-water in slots; `0xFFFFFFFF` = ⊤.
    pub high: u32,
    /// Effect set bits (abi-contract §4.1).
    pub effects: u16,
}

/// A parsed debug entry (name borrows from source).
#[derive(Clone, Debug)]
pub struct ParsedDebugEntry<'a> {
    pub sym_hash: u64,
    pub name: &'a [u8],
    pub net: i16,
    pub high: u32,
    pub effects: u16,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// Encode a table of word entries into `buf`.
///
/// Uses two passes over the entries and does not allocate:
/// - Pass 1: write the name table.
/// - Pass 2: write the record array (re-computes name offsets from the
///   name table written in pass 1).
///
/// Returns `Some(total_bytes)` on success, or `None` if `buf` is too small
/// or there are too many entries.
pub fn encode_into(buf: &mut [u8], entries: &[DebugEntry<'_>]) -> Option<usize> {
    let count = entries.len();
    if count > MAX_DEBUG_ENTRIES {
        return None;
    }

    // Compute total size.
    let mut name_table_size = 0usize;
    for e in entries.iter() {
        name_table_size += e.name.len() + 1; // null terminator
    }
    let name_table_padded = pad4(name_table_size);
    let total = 4 + name_table_padded + count * DEBUG_RECORD_SIZE as usize;
    if total > buf.len() {
        return None;
    }

    // Zero-fill.
    for b in buf[..total].iter_mut() {
        *b = 0;
    }

    // ---- Pass 1: write count + name table ----
    let count_u32 = count as u32;
    buf[0..4].copy_from_slice(&count_u32.to_le_bytes());

    let mut name_off = 4usize;
    for e in entries.iter() {
        buf[name_off..name_off + e.name.len()].copy_from_slice(e.name);
        name_off += e.name.len();
        buf[name_off] = 0; // null terminator
        name_off += 1;
    }

    // ---- Pass 2: write records ----
    name_off = 4; // rewind to walk name table again
    let mut rec_off = 4 + name_table_padded;
    for e in entries.iter() {
        // sym_hash
        buf[rec_off..rec_off + 8].copy_from_slice(&e.sym_hash.to_le_bytes());
        // name_off
        buf[rec_off + 8..rec_off + 12].copy_from_slice(&(name_off as u32).to_le_bytes());
        // net
        buf[rec_off + 12..rec_off + 14].copy_from_slice(&e.net.to_le_bytes());
        // high
        buf[rec_off + 14..rec_off + 18].copy_from_slice(&e.high.to_le_bytes());
        // effects
        buf[rec_off + 18..rec_off + 20].copy_from_slice(&e.effects.to_le_bytes());
        // bytes 20-23 are reserved (already zero from the fill above)

        name_off += e.name.len() + 1;
        rec_off += DEBUG_RECORD_SIZE as usize;
    }

    Some(total)
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decode the header of a `.lang.debug` section and return `(count, name_table_padded_size)`.
pub fn decode_header(data: &[u8]) -> Option<(u32, usize)> {
    if data.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes(data[0..4].try_into().ok()?);
    if count > MAX_DEBUG_ENTRIES as u32 {
        return None;
    }
    if count == 0 {
        return Some((0, 4));
    }

    // Scan the name table: find null terminators for each name.
    let mut pos = 4usize;
    for _ in 0..count {
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        if pos >= data.len() {
            return None;
        }
        pos += 1;
    }
    // name_table_padded = padded size of name table RELATIVE to offset 4.
    let name_table_size = pos - 4;
    let name_table_padded = pad4(name_table_size);
    Some((count, name_table_padded))
}

/// Read one word record by index.
pub fn read_entry(data: &[u8], index: u32) -> Option<ParsedDebugEntry<'_>> {
    let (count, name_table_padded) = decode_header(data)?;
    if index >= count {
        return None;
    }
    let rec_off = 4 + name_table_padded + (index as usize) * DEBUG_RECORD_SIZE as usize;
    let rec_end = rec_off + DEBUG_RECORD_SIZE as usize;
    if rec_end > data.len() {
        return None;
    }

    let sym_hash = u64::from_le_bytes(data[rec_off..rec_off + 8].try_into().ok()?);
    let name_off = u32::from_le_bytes(data[rec_off + 8..rec_off + 12].try_into().ok()?) as usize;
    let net = i16::from_le_bytes(data[rec_off + 12..rec_off + 14].try_into().ok()?);
    let high = u32::from_le_bytes(data[rec_off + 14..rec_off + 18].try_into().ok()?);
    let effects = u16::from_le_bytes(data[rec_off + 18..rec_off + 20].try_into().ok()?);

    // Validate name_off.
    if name_off >= data.len() {
        return None;
    }
    let name_slice = &data[name_off..];
    let end = name_slice
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_slice.len());
    let name = &data[name_off..name_off + end];

    Some(ParsedDebugEntry {
        sym_hash,
        name,
        net,
        high,
        effects,
    })
}

/// Iterate all entries, calling `f` for each.
pub fn for_each<F: FnMut(ParsedDebugEntry<'_>)>(data: &[u8], mut f: F) {
    let (count, _) = match decode_header(data) {
        Some(c) => c,
        None => return,
    };
    for i in 0..count {
        if let Some(entry) = read_entry(data, i) {
            f(entry);
        }
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

impl fmt::Debug for DebugEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugEntry")
            .field("sym_hash", &format_args!("{:#x}", self.sym_hash))
            .field(
                "name",
                &core::str::from_utf8(self.name).unwrap_or("<invalid>"),
            )
            .field("net", &self.net)
            .field("high", &self.high)
            .field("effects", &format_args!("{:#x}", self.effects))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::fnv1a_u64;

    fn make_entry(name: &str, net: i16, high: u32, effects: u16) -> DebugEntry<'_> {
        DebugEntry {
            sym_hash: fnv1a_u64(name.as_bytes()),
            name: name.as_bytes(),
            net,
            high,
            effects,
        }
    }

    fn roundtrip(entries: &[DebugEntry<'_>]) {
        let mut buf = [0u8; 65536];
        let n = encode_into(&mut buf, entries).unwrap();
        let data = &buf[..n];

        let (count, _) = decode_header(data).unwrap();
        assert_eq!(count as usize, entries.len());

        for i in 0..entries.len() {
            let parsed = read_entry(data, i as u32).unwrap();
            assert_eq!(parsed.sym_hash, entries[i].sym_hash);
            assert_eq!(parsed.name, entries[i].name);
            assert_eq!(parsed.net, entries[i].net);
            assert_eq!(parsed.high, entries[i].high);
            assert_eq!(parsed.effects, entries[i].effects);
        }
    }

    #[test]
    fn empty_section() {
        roundtrip(&[]);
    }

    #[test]
    fn single_entry() {
        roundtrip(&[make_entry("main", 1, 42, 0)]);
    }

    #[test]
    fn multiple_entries() {
        roundtrip(&[
            make_entry("main", 0, 128, 0),
            make_entry("helper", 2, 64, 1),
            make_entry("private-impl", -1, 0xFFFF_FFFF, 3),
        ]);
    }

    #[test]
    fn long_name() {
        roundtrip(&[make_entry(
            "this-is-a-very-long-word-name-that-exceeds-32-bytes",
            0,
            0,
            0,
        )]);
    }

    #[test]
    fn many_entries() {
        // Build owned names first, then reference them.
        let names: alloc::vec::Vec<alloc::string::String> =
            (0..100).map(|i| alloc::format!("word_{}", i)).collect();
        let entries: alloc::vec::Vec<DebugEntry<'_>> = names
            .iter()
            .enumerate()
            .map(|(i, n)| make_entry(n.as_str(), i as i16, (i * 10) as u32, 0))
            .collect();
        roundtrip(&entries);
    }

    #[test]
    fn decode_header_rejects_truncated() {
        assert!(decode_header(b"").is_none());
        assert!(decode_header(b"\x01\x00\x00\x00").is_none());
    }

    #[test]
    fn for_each_counts_correctly() {
        let entries = [make_entry("a", 0, 0, 0), make_entry("b", 1, 10, 2)];
        let mut buf = [0u8; 4096];
        let n = encode_into(&mut buf, &entries).unwrap();
        let mut count = 0usize;
        for_each(&buf[..n], |_| count += 1);
        assert_eq!(count, entries.len());
    }

    #[test]
    fn max_entries_rejected() {
        let entries = [make_entry("x", 0, 0, 0); MAX_DEBUG_ENTRIES + 1];
        let mut buf = [0u8; 1_000_000];
        assert!(encode_into(&mut buf, &entries).is_none());
    }
}
