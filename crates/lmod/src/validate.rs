//! `.lmod` container reader with full integrity validation.
//!
//! Canonical definition: module-format-and-loading.md §3, §6 (integrity);
//! loader algorithm steps 1–2 in §9.
//!
//! The reader is `#![no_std]`-safe and never panics on hostile input.
//! Every method returns `Result` or `Option`; the `parse` constructor runs
//! the full suite of checks from §9 steps 1–2 before exposing any data.

use crate::header::{self, LmodHeader, HEADER_SIZE, LMOD_MAGIC, FORMAT_VER};
use crate::reloc::{self, RelocEntry, RELOC_ENTRY_SIZE};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// A container-validation failure.
///
/// Every variant maps to `E_BAD_CONTAINER (5201)` in the module-format error
/// band.  The single-error-code approach matches the spec (§9 step 2) which
/// treats any structural violation as an untrusted-container rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error(pub u32);

impl Error {
    pub const BAD_CONTAINER: Self = Error(5201);
}

// ---------------------------------------------------------------------------
// Container — the parsed, validated view
// ---------------------------------------------------------------------------

/// A fully validated `.lmod` container.
///
/// Obtain one via [`Container::parse`]; after that all slice accessors are
/// infallible because the constructor verified every offset and length.
#[derive(Clone, Debug)]
pub struct Container<'a> {
    /// The complete container bytes (header + sections + relocs + trailer).
    data: &'a [u8],
    /// Decoded and validated header.
    hdr: LmodHeader,
}

impl<'a> Container<'a> {
    /// Parse and validate a `.lmod` container from raw bytes.
    ///
    /// Checks performed (in order, matching §9 steps 1–2):
    ///
    /// 1. Minimum length (≥ `HEADER_SIZE`).
    /// 2. Magic == `LMOD_MAGIC`.
    /// 3. Format version == `FORMAT_VER`.
    /// 4. Every section `(off, len)` is within `[0, total_len)`; `off ≥ HEADER_SIZE`.
    /// 5. No section overlaps another (monotonic ordering enforced).
    /// 6. Reloc and sig trailer bounds are within `total_len`.
    /// 7. `total_len` matches `data.len()`.
    ///
    /// Returns `Error::BAD_CONTAINER` on any failure.  Never panics.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        // 1. Minimum length.
        if data.len() < HEADER_SIZE as usize {
            return Err(Error::BAD_CONTAINER);
        }

        // 2–3. Parse header, check magic and version.
        let hdr = crate::header::decode_header(data).ok_or(Error::BAD_CONTAINER)?;
        if hdr.magic != LMOD_MAGIC {
            return Err(Error::BAD_CONTAINER);
        }
        if hdr.format_ver != FORMAT_VER {
            return Err(Error::BAD_CONTAINER);
        }

        let total = hdr.total_len as usize;

        // 7. total_len must match actual data length.
        if total != data.len() {
            return Err(Error::BAD_CONTAINER);
        }

        // 4. Validate every section range.
        // Helper: check `(off, len)` fits within `[0, total]`.
        let check_range = |off: u32, len: u32| -> Result<(), Error> {
            if len == 0 {
                return Ok(());
            }
            let end = off.checked_add(len).ok_or(Error::BAD_CONTAINER)?;
            if (end as usize) > total {
                return Err(Error::BAD_CONTAINER);
            }
            if off < HEADER_SIZE {
                // Sections must not overlap the header area.
                return Err(Error::BAD_CONTAINER);
            }
            Ok(())
        };

        check_range(hdr.modinfo_off, hdr.modinfo_len)?;
        check_range(hdr.code_off, hdr.code_len)?;
        check_range(hdr.rodata_off, hdr.rodata_len)?;
        check_range(hdr.data_off, hdr.data_len)?;

        // 6. Reloc and sig ranges.
        let reloc_bytes = (hdr.reloc_count as usize)
            .checked_mul(RELOC_ENTRY_SIZE as usize)
            .ok_or(Error::BAD_CONTAINER)?;
        if reloc_bytes > 0 {
            let reloc_end = (hdr.reloc_off as usize)
                .checked_add(reloc_bytes)
                .ok_or(Error::BAD_CONTAINER)?;
            if reloc_end > total {
                return Err(Error::BAD_CONTAINER);
            }
            if hdr.reloc_off < HEADER_SIZE {
                return Err(Error::BAD_CONTAINER);
            }
        }
        if hdr.sig_len > 0 {
            let sig_end = (hdr.sig_off as usize)
                .checked_add(hdr.sig_len as usize)
                .ok_or(Error::BAD_CONTAINER)?;
            if sig_end > total {
                return Err(Error::BAD_CONTAINER);
            }
            if hdr.sig_off < HEADER_SIZE {
                return Err(Error::BAD_CONTAINER);
            }
        }

        // 5. Check no overlaps between sections.
        // Build a sorted list of (start, end) ranges, then verify monotonicity.
        let mut ranges: [(u32, u32); 6] = [
            (0, HEADER_SIZE), // header (implicit)
            (hdr.modinfo_off, hdr.modinfo_off.saturating_add(hdr.modinfo_len)),
            (hdr.code_off, hdr.code_off.saturating_add(hdr.code_len)),
            (hdr.rodata_off, hdr.rodata_off.saturating_add(hdr.rodata_len)),
            (hdr.data_off, hdr.data_off.saturating_add(hdr.data_len)),
            (hdr.reloc_off, hdr.reloc_off.saturating_add(reloc_bytes as u32)),
        ];

        // Sort by start offset (insertion sort for tiny array).
        let mut i = 1;
        while i < ranges.len() {
            let mut j = i;
            while j > 0 && ranges[j - 1].0 > ranges[j].0 {
                ranges.swap(j - 1, j);
                j -= 1;
            }
            i += 1;
        }

        // Verify each range ends before the next starts.
        for w in ranges.windows(2) {
            let (_, end_a) = w[0];
            let (start_b, _) = w[1];
            if end_a > start_b && end_a != 0 && start_b != 0 {
                // Both ranges are non-empty and they overlap.
                return Err(Error::BAD_CONTAINER);
            }
        }

        // Ensure reloc entries don't overflow past total_len (redundant with
        // check_range but kept for defence in depth).
        if reloc_bytes > 0 {
            let reloc_end = (hdr.reloc_off as usize) + reloc_bytes;
            if reloc_end > total {
                return Err(Error::BAD_CONTAINER);
            }
        }

        Ok(Self { data, hdr })
    }

    // -----------------------------------------------------------------------
    // Accessors — all infallible once you have a `Container`.
    // -----------------------------------------------------------------------

    /// The validated header.
    pub fn header(&self) -> &LmodHeader {
        &self.hdr
    }

    /// The `.lang.modinfo` section bytes.
    pub fn modinfo(&self) -> &[u8] {
        if self.hdr.modinfo_len == 0 {
            return &[];
        }
        let start = self.hdr.modinfo_off as usize;
        &self.data[start..start + self.hdr.modinfo_len as usize]
    }

    /// The `.text` (code) section bytes.
    pub fn code(&self) -> &[u8] {
        if self.hdr.code_len == 0 {
            return &[];
        }
        let start = self.hdr.code_off as usize;
        &self.data[start..start + self.hdr.code_len as usize]
    }

    /// The `.rodata` section bytes.
    pub fn rodata(&self) -> &[u8] {
        if self.hdr.rodata_len == 0 {
            return &[];
        }
        let start = self.hdr.rodata_off as usize;
        &self.data[start..start + self.hdr.rodata_len as usize]
    }

    /// The `.data` section bytes (initialised image).
    pub fn data(&self) -> &[u8] {
        if self.hdr.data_len == 0 {
            return &[];
        }
        let start = self.hdr.data_off as usize;
        &self.data[start..start + self.hdr.data_len as usize]
    }

    /// BSS length (zero-fill region, not stored in the container).
    pub fn bss_len(&self) -> u32 {
        self.hdr.bss_len
    }

    /// Number of import relocation entries.
    pub fn reloc_count(&self) -> u32 {
        self.hdr.reloc_count
    }

    /// Read one import relocation entry by index.
    ///
    /// Returns `None` if `index >= reloc_count`.
    pub fn reloc_entry(&self, index: u32) -> Option<RelocEntry> {
        if index >= self.hdr.reloc_count {
            return None;
        }
        let entry_off = (self.hdr.reloc_off as usize)
            + (index as usize) * RELOC_ENTRY_SIZE as usize;
        crate::reloc::decode_entry(self.data, entry_off)
    }

    /// An iterator over all import relocation entries.
    pub fn reloc_iter(&'a self) -> RelocIter<'a> {
        RelocIter {
            container: self,
            next_index: 0,
        }
    }

    /// Raw byte slice of the reloc table region (for low-level access).
    pub fn reloc_raw(&self) -> &[u8] {
        let n = (self.hdr.reloc_count as usize) * RELOC_ENTRY_SIZE as usize;
        if n == 0 {
            return &[];
        }
        let start = self.hdr.reloc_off as usize;
        &self.data[start..start + n]
    }

    /// Total container length (same as `self.header().total_len`).
    pub fn total_len(&self) -> u32 {
        self.hdr.total_len
    }
}

// ---------------------------------------------------------------------------
// RelocIter — an iterator over import relocation entries
// ---------------------------------------------------------------------------

pub struct RelocIter<'a> {
    container: &'a Container<'a>,
    next_index: u32,
}

impl<'a> Iterator for RelocIter<'a> {
    type Item = RelocEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.container.reloc_entry(self.next_index);
        if entry.is_some() {
            self.next_index += 1;
        }
        entry
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Build a minimal valid .lmod container in a Vec.
    fn build_valid_container() -> Vec<u8> {
        let mut buf = vec![0u8; 1024];

        // Write a valid header using the encoder, then manually place sections.
        let layout = header::compute_layout(42, 32, 64, 16, 8, 0, 2);
        header::encode_header(&mut buf, &layout);

        // Write section data.
        let mi = buf.as_mut_slice();
        let modinfo_start = layout.modinfo_off as usize;
        // Write a minimal LangModInfo: magic (4) + ver (2) + flags (2) + abi_hash (8) = 16 bytes
        mi[modinfo_start..modinfo_start + 4].copy_from_slice(&[0x44, 0x4f, 0x4d, 0x4c]); // LMOD magic
        // version (2) and flags (2) are already zero from memset
        mi[modinfo_start + 8..modinfo_start + 16].copy_from_slice(&42u64.to_le_bytes()); // abi_hash

        let code_start = layout.code_off as usize;
        for i in 0..64 { mi[code_start + i] = 0xcc; }

        let rodata_start = layout.rodata_off as usize;
        for i in 0..16 { mi[rodata_start + i] = i as u8; }

        let data_start = layout.data_off as usize;
        for i in 0..8 { mi[data_start + i] = 0xff; }

        // Write reloc entries.
        let reloc_start = layout.reloc_off as usize;
        let mut e = [0u8; 16];
        e[0..4].copy_from_slice(&0u32.to_le_bytes());
        e[4..12].copy_from_slice(&0x1234u64.to_le_bytes());
        e[12] = 1;
        mi[reloc_start..reloc_start + 16].copy_from_slice(&e);
        let mut e = [0u8; 16];
        e[0..4].copy_from_slice(&42u32.to_le_bytes());
        e[4..12].copy_from_slice(&0x5678u64.to_le_bytes());
        e[12] = 2;
        mi[reloc_start + 16..reloc_start + 32].copy_from_slice(&e);

        buf.truncate(layout.total_len as usize);
        buf
    }

    #[test]
    fn valid_container_parses() {
        let bytes = build_valid_container();
        let c = Container::parse(&bytes).unwrap();
        assert_eq!(c.header().abi_hash, 42);
        assert_eq!(c.header().modinfo_len, 32);
        assert_eq!(c.header().code_len, 64);
        assert_eq!(c.header().reloc_count, 2);
    }

    #[test]
    fn valid_container_slices_match_header() {
        let bytes = build_valid_container();
        let c = Container::parse(&bytes).unwrap();

        assert_eq!(c.modinfo().len(), 32);
        assert_eq!(c.code().len(), 64);
        assert_eq!(c.rodata().len(), 16);
        assert_eq!(c.data().len(), 8);

        // First byte of code must be 0xcc.
        assert_eq!(c.code()[0], 0xcc);
    }

    #[test]
    fn valid_container_reloc_iter() {
        let bytes = build_valid_container();
        let c = Container::parse(&bytes).unwrap();

        let entries: Vec<RelocEntry> = c.reloc_iter().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sym_hash, 0x1234);
        assert_eq!(entries[0].kind, 1);
        assert_eq!(entries[1].site_off, 42);
        assert_eq!(entries[1].sym_hash, 0x5678);
    }

    #[test]
    fn reloc_entry_out_of_range() {
        let bytes = build_valid_container();
        let c = Container::parse(&bytes).unwrap();
        assert!(c.reloc_entry(0).is_some());
        assert!(c.reloc_entry(1).is_some());
        assert!(c.reloc_entry(2).is_none()); // only 2 entries
    }

    #[test]
    fn empty_container_rejected() {
        assert!(Container::parse(b"").is_err());
    }

    #[test]
    fn short_container_rejected() {
        assert!(Container::parse(b"LMOD").is_err());
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = build_valid_container();
        bytes[0] = 0x00; // corrupt magic
        assert!(Container::parse(&bytes).is_err());
    }

    #[test]
    fn bad_version_rejected() {
        let mut bytes = build_valid_container();
        bytes[4] = 99; // corrupt format_ver
        assert!(Container::parse(&bytes).is_err());
    }

    #[test]
    fn truncated_data_rejected() {
        let bytes = build_valid_container();
        let truncated = &bytes[..bytes.len() / 2];
        assert!(Container::parse(truncated).is_err());
    }

    #[test]
    fn overlapping_sections_rejected() {
        let mut bytes = build_valid_container();
        let mi_end = u32::from_le_bytes(bytes[20..24].try_into().unwrap())
            + u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        // Set code_off to mi_end - 4 (overlap)
        let overlapping_off = mi_end - 4;
        bytes[28..32].copy_from_slice(&overlapping_off.to_le_bytes());
        assert!(Container::parse(&bytes).is_err());
    }

    #[test]
    fn section_off_before_header_rejected() {
        let mut bytes = build_valid_container();
        bytes[28..32].copy_from_slice(&4u32.to_le_bytes()); // code_off inside header
        assert!(Container::parse(&bytes).is_err());
    }

    #[test]
    fn total_len_mismatch_rejected() {
        let mut bytes = build_valid_container();
        let total = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        bytes[16..20].copy_from_slice(&(total + 1).to_le_bytes());
        assert!(Container::parse(&bytes).is_err());
    }

    #[test]
    fn overflow_add_is_safe() {
        // Section with off = u32::MAX should not panic.
        let mut bytes = build_valid_container();
        bytes[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Container::parse(&bytes).is_err());
    }

    #[test]
    fn reloc_iter_empty_when_no_relocs() {
        let h = header::compute_layout(0, 8, 16, 0, 0, 0, 0);
        let total = h.total_len as usize;
        let mut buf = vec![0u8; total];
        header::encode_header(&mut buf, &h);
        let mi = &mut buf[h.modinfo_off as usize..];
        mi[0..4].copy_from_slice(&crate::modinfo::LMOD_MAGIC.to_le_bytes());
        mi[8..16].copy_from_slice(&0u64.to_le_bytes());
        let c = Container::parse(&buf).unwrap();
        assert_eq!(c.reloc_count(), 0);
        assert!(c.reloc_iter().next().is_none());
        assert!(c.reloc_entry(0).is_none());
    }

    #[test]
    fn reloc_entry_padding_is_ignored() {
        // Reloc entries have 3 bytes of padding after the kind byte.
        // Verify that non-zero padding is accepted.
        let mut buf = build_valid_container();
        let reloc_start = u32::from_le_bytes(buf[56..60].try_into().unwrap()) as usize;
        // Set padding byte to 0xff — should still parse.
        buf[reloc_start + 13] = 0xff;
        buf[reloc_start + 14] = 0xff;
        buf[reloc_start + 15] = 0xff;
        let c = Container::parse(&buf).unwrap();
        let entry = c.reloc_entry(0).unwrap();
        assert_eq!(entry.site_off, 0);
        assert_eq!(entry.sym_hash, 0x1234);
        assert_eq!(entry.kind, 1);
    }

    #[test]
    fn sig_trailer_within_bounds_ok() {
        // Container with a (zero-length) sig area is valid.
        let mut buf = build_valid_container();
        // sig_off is already set by compute_layout, sig_len = 0.
        let c = Container::parse(&buf).unwrap();
        assert_eq!(c.header().sig_len, 0);
    }

    // -----------------------------------------------------------------------
    // Fuzz-style: random malformed containers must never panic
    // -----------------------------------------------------------------------

    #[test]
    fn fuzz_random_mutations_no_panic() {
        let base = build_valid_container();
        let mut rng = 12345u64;
        for _ in 0..256 {
            let mut bytes = base.clone();
            // Mutate 1–5 random bytes.
            let n_mutations = (rng % 5) + 1;
            for _ in 0..n_mutations {
                let idx = (rng as usize) % bytes.len();
                bytes[idx] = bytes[idx].wrapping_add((rng & 0xff) as u8);
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            }
            // Must never panic, must return Err or Ok.
            let _result = Container::parse(&bytes);
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        }
    }

    #[test]
    fn fuzz_all_zeros_no_panic() {
        for size in [0, 1, 8, 16, 32, 64, 72, 128, 256, 1024] {
            let bytes = vec![0u8; size];
            let _result = Container::parse(&bytes);
        }
    }

    #[test]
    fn fuzz_all_ffs_no_panic() {
        for size in [0, 1, 72, 128, 256] {
            let bytes = vec![0xffu8; size];
            let _result = Container::parse(&bytes);
        }
    }

    #[test]
    fn fuzz_single_bit_flips_no_panic() {
        let base = build_valid_container();
        for i in 0..base.len() {
            let mut bytes = base.clone();
            bytes[i] ^= 0x01; // flip one bit
            let _result = Container::parse(&bytes);
        }
    }

    #[test]
    fn fuzz_truncated_every_offset_no_panic() {
        let base = build_valid_container();
        // Test every truncation point from 0 to full length.
        for len in 0..=base.len() {
            let _result = Container::parse(&base[..len]);
        }
    }

    // -----------------------------------------------------------------------
    // End-to-end: packages produced by lmod-pack must be valid
    // -----------------------------------------------------------------------

    #[test]
    fn zero_length_sections_ok() {
        let header = header::compute_layout(0, 8, 32, 0, 0, 0, 0);
        let total = header.total_len as usize;
        let mut buf = vec![0u8; total];
        header::encode_header(&mut buf, &header);
        // Write minimal modinfo.
        let mi = &mut buf[header.modinfo_off as usize..];
        mi[0..4].copy_from_slice(&crate::modinfo::LMOD_MAGIC.to_le_bytes());
        mi[8..16].copy_from_slice(&0u64.to_le_bytes());
        // Write code.
        let code = &mut buf[header.code_off as usize..];
        code[0] = 0xc3; // ret

        let c = Container::parse(&buf).unwrap();
        assert_eq!(c.rodata().len(), 0);
        assert_eq!(c.data().len(), 0);
        assert_eq!(c.reloc_count(), 0);
        assert_eq!(c.code().len(), 32);
    }
}
