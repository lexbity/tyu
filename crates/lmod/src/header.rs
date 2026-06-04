//! `.lmod` container header codec.
//!
//! Canonical definition: module-format-and-loading.md §3.

pub const LMOD_MAGIC: u32 = 0x4c4d4f44; // "LMOD"

/// Current format version.
///
/// - 2: original (no encryption envelope).
/// - 3: adds optional `EncHeader` after the container header (inside the
///      signed region) and before modinfo.  A v3 container with
///      `enc_header_len == 0` is byte-identical to v2 except this field.
pub const FORMAT_VER: u16 = 3;

/// Size of the fixed container header in bytes.
pub const HEADER_SIZE: u32 = 72;

/// Container header flags (bits in `LmodHeader.flags`).
///
/// Bit assignments per module-format-and-loading.md §3:
///   bit 0: signed     — signature/MAC trailer present
///   bit 1: encrypted  — container is encrypted-at-rest
pub const LMOD_FLAG_SIGNED: u16 = 1 << 0;
pub const LMOD_FLAG_ENCRYPTED: u16 = 1 << 1;

// ---------------------------------------------------------------------------
// In-memory representation
// ---------------------------------------------------------------------------

/// The on-wire `.lmod` container header.
///
/// Every field is little-endian on the wire.  All offsets are byte offsets
/// from the start of the container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LmodHeader {
    pub magic: u32,
    pub format_ver: u16,
    pub flags: u16,
    pub abi_hash: u64,
    pub total_len: u32,
    pub modinfo_off: u32,
    pub modinfo_len: u32,
    pub code_off: u32,
    pub code_len: u32,
    pub rodata_off: u32,
    pub rodata_len: u32,
    pub data_off: u32,
    pub data_len: u32,
    pub bss_len: u32,
    pub reloc_off: u32,
    pub reloc_count: u32,
    pub sig_off: u32,
    pub sig_len: u32,
}

impl LmodHeader {
    pub const fn new() -> Self {
        Self {
            magic: LMOD_MAGIC,
            format_ver: FORMAT_VER,
            flags: 0,
            abi_hash: 0,
            total_len: 0,
            modinfo_off: HEADER_SIZE,
            modinfo_len: 0,
            code_off: 0,
            code_len: 0,
            rodata_off: 0,
            rodata_len: 0,
            data_off: 0,
            data_len: 0,
            bss_len: 0,
            reloc_off: 0,
            reloc_count: 0,
            sig_off: 0,
            sig_len: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Encode / decode
// ---------------------------------------------------------------------------

/// Write a header into `buf` (which must be at least `HEADER_SIZE` bytes).
pub fn encode_header(buf: &mut [u8], h: &LmodHeader) {
    let off = 0usize;
    buf[off..off + 4].copy_from_slice(&h.magic.to_le_bytes());
    buf[off + 4..off + 6].copy_from_slice(&h.format_ver.to_le_bytes());
    buf[off + 6..off + 8].copy_from_slice(&h.flags.to_le_bytes());
    buf[off + 8..off + 16].copy_from_slice(&h.abi_hash.to_le_bytes());
    buf[off + 16..off + 20].copy_from_slice(&h.total_len.to_le_bytes());
    buf[off + 20..off + 24].copy_from_slice(&h.modinfo_off.to_le_bytes());
    buf[off + 24..off + 28].copy_from_slice(&h.modinfo_len.to_le_bytes());
    buf[off + 28..off + 32].copy_from_slice(&h.code_off.to_le_bytes());
    buf[off + 32..off + 36].copy_from_slice(&h.code_len.to_le_bytes());
    buf[off + 36..off + 40].copy_from_slice(&h.rodata_off.to_le_bytes());
    buf[off + 40..off + 44].copy_from_slice(&h.rodata_len.to_le_bytes());
    buf[off + 44..off + 48].copy_from_slice(&h.data_off.to_le_bytes());
    buf[off + 48..off + 52].copy_from_slice(&h.data_len.to_le_bytes());
    buf[off + 52..off + 56].copy_from_slice(&h.bss_len.to_le_bytes());
    buf[off + 56..off + 60].copy_from_slice(&h.reloc_off.to_le_bytes());
    buf[off + 60..off + 64].copy_from_slice(&h.reloc_count.to_le_bytes());
    buf[off + 64..off + 68].copy_from_slice(&h.sig_off.to_le_bytes());
    buf[off + 68..off + 72].copy_from_slice(&h.sig_len.to_le_bytes());
}

/// Parse a header from exactly `HEADER_SIZE` bytes.
///
/// Returns `None` on bad magic or version.
pub fn decode_header(bytes: &[u8]) -> Option<LmodHeader> {
    if bytes.len() < HEADER_SIZE as usize {
        return None;
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    if magic != LMOD_MAGIC {
        return None;
    }
    Some(LmodHeader {
        magic,
        format_ver: u16::from_le_bytes(bytes[4..6].try_into().ok()?),
        flags: u16::from_le_bytes(bytes[6..8].try_into().ok()?),
        abi_hash: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        total_len: u32::from_le_bytes(bytes[16..20].try_into().ok()?),
        modinfo_off: u32::from_le_bytes(bytes[20..24].try_into().ok()?),
        modinfo_len: u32::from_le_bytes(bytes[24..28].try_into().ok()?),
        code_off: u32::from_le_bytes(bytes[28..32].try_into().ok()?),
        code_len: u32::from_le_bytes(bytes[32..36].try_into().ok()?),
        rodata_off: u32::from_le_bytes(bytes[36..40].try_into().ok()?),
        rodata_len: u32::from_le_bytes(bytes[40..44].try_into().ok()?),
        data_off: u32::from_le_bytes(bytes[44..48].try_into().ok()?),
        data_len: u32::from_le_bytes(bytes[48..52].try_into().ok()?),
        bss_len: u32::from_le_bytes(bytes[52..56].try_into().ok()?),
        reloc_off: u32::from_le_bytes(bytes[56..60].try_into().ok()?),
        reloc_count: u32::from_le_bytes(bytes[60..64].try_into().ok()?),
        sig_off: u32::from_le_bytes(bytes[64..68].try_into().ok()?),
        sig_len: u32::from_le_bytes(bytes[68..72].try_into().ok()?),
    })
}

// ---------------------------------------------------------------------------
// Layout computation
// ---------------------------------------------------------------------------

/// Compute the placement of sections within a `.lmod` container.
///
/// `enc_header_len` is the size of the encryption envelope header (0 when
/// not encrypted).  When non-zero, the enc-header is placed immediately
/// after the fixed container header and inside the signed region.
///
/// Returns a fully populated `LmodHeader` with all offsets and lengths set,
/// plus the total container size.  The caller can then iterate:
///   1. Write `HEADER_SIZE` bytes of the encoded header.
///   2. If `enc_header_len > 0`, write the enc-header at offset `HEADER_SIZE`.
///   3. Write `modinfo` at `header.modinfo_off`.
///   4. Write code at `header.code_off`, rodata, data, etc.
pub fn compute_layout(
    abi_hash: u64,
    modinfo_len: u32,
    code_len: u32,
    rodata_len: u32,
    data_len: u32,
    bss_len: u32,
    reloc_count: u32,
    enc_header_len: u32,
) -> LmodHeader {
    let mut h = LmodHeader::new();
    h.abi_hash = abi_hash;

    let mut off = HEADER_SIZE;
    // Enc-header sits immediately after the fixed header (inside signed region).
    off += enc_header_len;

    h.modinfo_off = off;
    h.modinfo_len = modinfo_len;
    off += modinfo_len;
    off = (off + 3) & !3; // align 4

    h.code_off = off;
    h.code_len = code_len;
    off += code_len;
    off = (off + 7) & !7; // align 8

    h.rodata_off = off;
    h.rodata_len = rodata_len;
    off += rodata_len;
    off = (off + 7) & !7; // align 8

    h.data_off = off;
    h.data_len = data_len;
    off += data_len;
    off = (off + 7) & !7; // align 8

    h.bss_len = bss_len;

    h.reloc_off = off;
    h.reloc_count = reloc_count;
    off += reloc_count * crate::reloc::RELOC_ENTRY_SIZE;
    off = (off + 7) & !7; // align 8

    h.sig_off = off;
    h.sig_len = 0;
    off += h.sig_len;

    h.total_len = off;
    h
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn header_roundtrip() {
        let h = LmodHeader {
            magic: LMOD_MAGIC,
            format_ver: FORMAT_VER,
            flags: 0,
            abi_hash: 0xdeadbeefcafebabe,
            total_len: 1234,
            modinfo_off: 72,
            modinfo_len: 64,
            code_off: 144,
            code_len: 256,
            rodata_off: 400,
            rodata_len: 32,
            data_off: 440,
            data_len: 16,
            bss_len: 128,
            reloc_off: 464,
            reloc_count: 5,
            sig_off: 0,
            sig_len: 0,
        };
        let mut buf = [0u8; HEADER_SIZE as usize];
        encode_header(&mut buf, &h);
        let decoded = decode_header(&buf).unwrap();
        assert_eq!(decoded, h);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let buf = [0u8; HEADER_SIZE as usize];
        assert!(decode_header(&buf).is_none());
    }

    #[test]
    fn decode_rejects_short() {
        assert!(decode_header(b"").is_none());
    }

    #[test]
    fn compute_layout_basic() {
        let h = compute_layout(42, 32, 128, 16, 8, 0, 2, 0);
        assert_eq!(h.magic, LMOD_MAGIC);
        assert_eq!(h.abi_hash, 42);
        assert_eq!(h.modinfo_off, HEADER_SIZE);
        assert_eq!(h.modinfo_len, 32);
        // code starts after modinfo + 4-byte align
        assert_eq!(h.code_off, HEADER_SIZE + 32);
        assert_eq!(h.code_len, 128);
        // Relocs start after data + 8-byte align
        assert!(h.reloc_count > 0);
        assert!(h.total_len > h.code_off + 128);
    }

    #[test]
    fn compute_layout_total_len() {
        let h = compute_layout(0, 16, 64, 32, 8, 0, 1, 0);
        // Verifiable invariants:
        // header = 72
        // modinfo at 72, len 16 → ends at 88
        // align 4 → 88
        // code at 88, len 64 → ends at 152
        // align 8 → 152
        // rodata at 152, len 32 → ends at 184
        // align 8 → 184
        // data at 184, len 8 → ends at 192
        // align 8 → 192
        // reloc at 192, count 1, entry size 16 → ends at 208
        // align 8 → 208
        // sig off 208, len 0
        // total = 208
        assert_eq!(h.total_len, 208);
    }

    #[test]
    fn flags_roundtrip() {
        for (name, flag) in &[("SIGNED", LMOD_FLAG_SIGNED), ("ENCRYPTED", LMOD_FLAG_ENCRYPTED)] {
            let mut h = LmodHeader::new();
            h.flags = *flag;
            let mut buf = vec![0u8; HEADER_SIZE as usize];
            encode_header(&mut buf, &h);
            let decoded = decode_header(&buf).unwrap();
            assert_eq!(
                decoded.flags & *flag,
                *flag,
                "flag {} should survive round-trip",
                name
            );
        }
    }

    #[test]
    fn compute_layout_empty_sections() {
        // Some modules have no rodata or data.
        let h = compute_layout(99, 8, 32, 0, 0, 0, 0, 0);
        assert_eq!(h.rodata_len, 0);
        assert_eq!(h.data_len, 0);
        assert_eq!(h.reloc_count, 0);
        // When rodata_len = 0, rodata_off = code_off + code_len (no gap).
        assert_eq!(h.rodata_off, h.code_off + h.code_len);
        // When data_len = 0, data_off = rodata_off (same position).
        assert_eq!(h.data_off, h.rodata_off);
    }
}
