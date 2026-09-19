//! `.lang.modinfo` wire-format codec.
//!
//! Canonical definition: abi-contract.md §3–§4.
//! Companion container layout: module-format-and-loading.md §3–§4.

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const LMOD_MAGIC: u32 = 0x4c4d4f44; // "LMOD"

/// Version of the `LangModInfo` struct (design doc §5.5, decision D-12).
///
/// v4 (P6): the header grows 32 → 48 bytes with `aperture_count`,
/// `res_meta_count`, and `platform_hash`, and a aperture-use table follows the
/// res-meta entries. `abi_hash` folds this version (abi_hash.rs), so a v3
/// module never decodes as v4 — the loader rejects it up front (E5224).
pub const MODINFO_VER: u16 = 4;

pub const MODINFO_HEADER_SIZE: u32 = 48;

pub const EXPORT_ENTRY_SIZE: u32 = 16; // u64 sym_hash + u32 name_off + u32 value_off
pub const IMPORT_ENTRY_SIZE: u32 = 12; // u64 sym_hash + u32 name_off
pub const WORD_META_SIZE: u32 = 16; // u64 sym_hash + u16 effects + u16 requires_caps + u32 stack_bound
pub const RES_META_SIZE: u32 = 16; // u64 res_hash + u8 sharing_class + u8[3] _pad + u32 lock_prim
pub const APERTURE_USE_ENTRY_SIZE: u32 = 16; // u64 name_hash + u32 size + u16 aperture_id + u8 access_mask + u8 _pad

/// Fused per-aperture access-mask bits (design doc §5.5). One source of truth:
/// these are re-exported from `ir` so the module format, the compiler, and the
/// loader all agree on the wire bits.
pub use ir::{
    ACCESS_EFFECTFUL_READ as ACCESS_EFFECTFUL_READ, ACCESS_READ as ACCESS_READ,
    ACCESS_W1C as ACCESS_W1C, ACCESS_W1S as ACCESS_W1S, ACCESS_WRITE as ACCESS_WRITE,
};

/// Fixed header field offsets (little-endian). v4 header = 48 bytes:
/// `0 magic · 4 modinfo_ver · 6 flags · 8 abi_hash · 16 name_off · 20 name_len
///  · 24 export_count · 28 import_count · 32 aperture_count · 34 res_meta_count
///  · 36 platform_hash · 44 _pad`.
pub const HDR_APERTURE_COUNT_OFF: usize = 32;
pub const HDR_RES_META_COUNT_OFF: usize = 34;
pub const HDR_PLATFORM_HASH_OFF: usize = 36;

/// Flags for the `LangModInfo.flags` field.
pub const MODINFO_FLAG_HAS_ISR: u16 = 0x0001;

// ---------------------------------------------------------------------------
// Entry types
// ---------------------------------------------------------------------------

/// An exported symbol, as passed to the encoder.
#[derive(Clone, Copy)]
pub struct ExportEntry<'a> {
    pub sym_hash: u64,
    /// Name bytes (must not be empty; should not contain NUL).
    pub name: &'a [u8],
    pub effects: u16,
    pub requires_caps: u16,
    pub stack_bound: u32,
}

/// An imported symbol, as passed to the encoder.
#[derive(Clone, Copy)]
pub struct ImportEntry<'a> {
    pub sym_hash: u64,
    /// Name bytes (must not be empty; should not contain NUL).
    pub name: &'a [u8],
}

/// A resource sharing-class entry, as passed to the encoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResMetaEntry {
    pub res_hash: u64,
    pub sharing_class: u8,
    pub lock_prim: u32,
}

/// A aperture-use table entry (design doc §5.5): one aperture the module touches.
///
/// `name_hash` is FNV-1a-64 of the aperture's board name — the stable identity
/// the loader matches against its board's aperture table. `aperture_id` is the
/// module's local id for this aperture (equal to the descriptor aperture id, which
/// the compiler uses in `MmioPlace` and relocations). `size` is the aperture
/// size the module was compiled against; `access_mask` fuses the accesses the
/// module performs (bits in `ACCESS_*`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApertureUseEntry {
    pub name_hash: u64,
    pub size: u32,
    pub aperture_id: u16,
    pub access_mask: u8,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn pad4(n: u32) -> u32 {
    (n + 3) & !3
}

/// Write a u32 in little-endian at position `off` in `buf`.
#[inline]
fn poke_u32(buf: &mut [u8], off: usize, v: u32) {
    let le = v.to_le_bytes();
    buf[off..off + 4].copy_from_slice(&le);
}

/// Write a u16 in little-endian at position `off` in `buf`.
#[inline]
fn poke_u16(buf: &mut [u8], off: usize, v: u16) {
    let le = v.to_le_bytes();
    buf[off..off + 2].copy_from_slice(&le);
}

/// Write a u64 in little-endian at position `off` in `buf`.
#[inline]
fn poke_u64(buf: &mut [u8], off: usize, v: u64) {
    let le = v.to_le_bytes();
    buf[off..off + 8].copy_from_slice(&le);
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// Encode a complete `LangModInfo` into `buf`.
///
/// Returns the number of bytes written on success, or `None` if `buf` is
/// too small.
pub fn encode_into<'a>(
    buf: &mut [u8],
    module_name: &[u8],
    exports: &[ExportEntry<'a>],
    imports: &[ImportEntry<'a>],
    abi_hash_val: u64,
    flags: u16,
    res_metas: &[ResMetaEntry],
    platform_hash: u64,
    apertures: &[ApertureUseEntry],
) -> Option<usize> {
    let export_count = exports.len() as u32;
    let import_count = imports.len() as u32;

    // Bounds check: the encoder uses fixed-size arrays of 64 elements.
    if export_count > 64 || import_count > 64 {
        return None;
    }
    if apertures.len() > 8 {
        return None;
    }

    let res_count = res_metas.len() as u32;
    let aperture_count = apertures.len() as u32;

    // --- Compute total size ---
    let mut total = MODINFO_HEADER_SIZE;

    total += module_name.len() as u32;
    total = pad4(total);
    for e in exports.iter() {
        total += e.name.len() as u32 + 1;
    }
    total = pad4(total);
    for i in imports.iter() {
        total += i.name.len() as u32 + 1;
    }
    total = pad4(total);
    total += export_count * EXPORT_ENTRY_SIZE;
    total += import_count * IMPORT_ENTRY_SIZE;
    total += export_count * WORD_META_SIZE;
    total += res_count * RES_META_SIZE;
    total += aperture_count * APERTURE_USE_ENTRY_SIZE;

    if (total as usize) > buf.len() {
        return None;
    }

    // Clear the used portion so padding bytes are zero.
    for b in buf[..total as usize].iter_mut() {
        *b = 0;
    }

    let mut off: usize = 0;

    // 1. Fixed header (48 bytes)
    poke_u32(buf, off, LMOD_MAGIC);
    off += 4;
    poke_u16(buf, off, MODINFO_VER);
    off += 2;
    poke_u16(buf, off, flags);
    off += 2; // flags
    poke_u64(buf, off, abi_hash_val);
    off += 8; // abi_hash (abi-contract §5)
    let name_off_pos = off;
    poke_u32(buf, off, 0);
    off += 4; // placeholder
    let name_len_pos = off;
    poke_u32(buf, off, 0);
    off += 4; // placeholder
    poke_u32(buf, off, export_count);
    off += 4;
    poke_u32(buf, off, import_count);
    off += 4;
    // v4 additions (design doc §5.5)
    poke_u16(buf, off, aperture_count as u16);
    off += 2;
    poke_u16(buf, off, res_count as u16);
    off += 2;
    poke_u64(buf, off, platform_hash);
    off += 8;
    off += 4; // _pad (reserved)
    // header = 48 bytes

    // 2. Name table — module name
    let module_name_off = off as u32;
    buf[off..off + module_name.len()].copy_from_slice(module_name);
    off += module_name.len();
    off = pad4(off as u32) as usize;

    // Export names (null-terminated)
    let mut export_name_offs: [u32; 64] = [0; 64];
    for (i, e) in exports.iter().enumerate() {
        export_name_offs[i] = off as u32;
        buf[off..off + e.name.len()].copy_from_slice(e.name);
        off += e.name.len();
        buf[off] = 0;
        off += 1;
    }
    off = pad4(off as u32) as usize;

    // Import names (null-terminated)
    let mut import_name_offs: [u32; 64] = [0; 64];
    for (i, imp) in imports.iter().enumerate() {
        import_name_offs[i] = off as u32;
        buf[off..off + imp.name.len()].copy_from_slice(imp.name);
        off += imp.name.len();
        buf[off] = 0;
        off += 1;
    }
    off = pad4(off as u32) as usize;

    // 3. Export entries
    let export_entries_off = off as u32;
    let word_meta_base =
        export_entries_off + export_count * EXPORT_ENTRY_SIZE + import_count * IMPORT_ENTRY_SIZE;

    for (i, e) in exports.iter().enumerate() {
        let value_off = word_meta_base + i as u32 * WORD_META_SIZE;
        poke_u64(buf, off, e.sym_hash);
        off += 8;
        poke_u32(buf, off, export_name_offs[i]);
        off += 4;
        poke_u32(buf, off, value_off);
        off += 4;
    }

    // 4. Import entries
    for (i, imp) in imports.iter().enumerate() {
        poke_u64(buf, off, imp.sym_hash);
        off += 8;
        poke_u32(buf, off, import_name_offs[i]);
        off += 4;
    }

    // 5. Word meta entries
    for e in exports.iter() {
        poke_u64(buf, off, e.sym_hash);
        off += 8;
        poke_u16(buf, off, e.effects);
        off += 2;
        poke_u16(buf, off, e.requires_caps);
        off += 2;
        poke_u32(buf, off, e.stack_bound);
        off += 4;
    }

    // 5b. Resource meta entries
    for r in res_metas.iter() {
        poke_u64(buf, off, r.res_hash);
        off += 8;
        buf[off] = r.sharing_class;
        off += 1;
        buf[off..off + 3].fill(0);
        off += 3;
        poke_u32(buf, off, r.lock_prim);
        off += 4;
    }

    // 5c. Aperture-use entries (design doc §5.5)
    for w in apertures.iter() {
        poke_u64(buf, off, w.name_hash);
        off += 8;
        poke_u32(buf, off, w.size);
        off += 4;
        poke_u16(buf, off, w.aperture_id);
        off += 2;
        buf[off] = w.access_mask;
        off += 1;
        buf[off] = 0; // _pad
        off += 1;
    }

    // 6. Patch header offsets
    poke_u32(buf, name_off_pos, module_name_off);
    poke_u32(buf, name_len_pos, module_name.len() as u32);

    Some(off)
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decoded `LangModInfo`, suitable for loader consumption.
pub struct ModInfo<'a> {
    pub module_name: &'a [u8],
    pub module_name_off: u32,
    pub module_name_len: u32,
    pub export_count: u32,
    pub import_count: u32,
    pub abi_hash: u64,
    pub flags: u16,
    /// `MODINFO_VER` as encoded in the artifact (v4). The loader rejects
    /// anything else up front (E5224) — never decodes across versions.
    pub modinfo_ver: u16,
    /// Number of aperture-use entries (design doc §5.5).
    pub aperture_count: u32,
    /// Number of resource-meta entries.
    pub res_meta_count: u32,
    /// The module's `platform_hash` (0 = unplatformed module; E5220 check).
    pub platform_hash: u64,
}

impl ModInfo<'_> {
    /// Returns `true` if the module declares an `@interrupt` binding.
    pub fn has_isr(&self) -> bool {
        self.flags & MODINFO_FLAG_HAS_ISR != 0
    }
}

/// Decode a `LangModInfo` header from raw bytes.
///
/// Returns `None` if the bytes are malformed (bad magic, bad version,
/// truncated, or structurally out-of-bounds).
pub fn decode(data: &[u8]) -> Option<ModInfo<'_>> {
    if data.len() < MODINFO_HEADER_SIZE as usize {
        return None;
    }
    let magic = u32::from_le_bytes(data[0..4].try_into().ok()?);
    if magic != LMOD_MAGIC {
        return None;
    }
    let modinfo_ver = u16::from_le_bytes(data[4..6].try_into().ok()?);
    let flags = u16::from_le_bytes(data[6..8].try_into().ok()?);
    let abi_hash = u64::from_le_bytes(data[8..16].try_into().ok()?);
    let name_off = u32::from_le_bytes(data[16..20].try_into().ok()?) as usize;
    let name_len = u32::from_le_bytes(data[20..24].try_into().ok()?) as usize;
    let export_count = u32::from_le_bytes(data[24..28].try_into().ok()?);
    let import_count = u32::from_le_bytes(data[28..32].try_into().ok()?);
    // v4 fields (design doc §5.5). A v3 artifact is exactly 32 bytes of header
    // with different semantics at these offsets — we still read them, but the
    // loader's version gate (E5224) runs before any structural use.
    let aperture_count = u16::from_le_bytes(data[32..34].try_into().ok()?) as u32;
    let res_meta_count = u16::from_le_bytes(data[34..36].try_into().ok()?) as u32;
    let platform_hash = u64::from_le_bytes(data[36..44].try_into().ok()?);

    if name_off + name_len > data.len() {
        return None;
    }
    let module_name = &data[name_off..name_off + name_len];
    Some(ModInfo {
        module_name,
        module_name_off: name_off as u32,
        module_name_len: name_len as u32,
        export_count,
        import_count,
        abi_hash,
        flags,
        modinfo_ver,
        aperture_count,
        res_meta_count,
        platform_hash,
    })
}

/// A single export entry decoded from raw bytes (name borrows from source).
#[derive(Clone, Debug)]
pub struct ParsedExport<'a> {
    pub sym_hash: u64,
    pub name: &'a [u8],
    pub value_off: u32,
}

/// Compute the byte offset of the export entries array within the modinfo
/// data, or `None` if the data is truncated.
pub fn export_entries_offset(data: &[u8]) -> Option<usize> {
    let hdr = decode(data)?;
    let mut off = MODINFO_HEADER_SIZE as usize;
    off += hdr.module_name_len as usize;
    off = (off + 3) & !3;
    for _ in 0..hdr.export_count {
        while off < data.len() && data[off] != 0 {
            off += 1;
        }
        if off >= data.len() {
            return None;
        }
        off += 1;
    }
    off = (off + 3) & !3;
    for _ in 0..hdr.import_count {
        while off < data.len() && data[off] != 0 {
            off += 1;
        }
        if off >= data.len() {
            return None;
        }
        off += 1;
    }
    off = (off + 3) & !3;
    if off + (hdr.export_count as usize) * EXPORT_ENTRY_SIZE as usize > data.len() {
        return None;
    }
    Some(off)
}

/// Read one export entry by index.
pub fn read_export<'a>(data: &'a [u8], index: u32) -> Option<ParsedExport<'a>> {
    let hdr = decode(data)?;
    if index >= hdr.export_count {
        return None;
    }
    let entries_off = export_entries_offset(data)?;
    let entry_off = entries_off + (index as usize) * EXPORT_ENTRY_SIZE as usize;
    let sym_hash = u64::from_le_bytes(data[entry_off..entry_off + 8].try_into().ok()?);
    let name_off =
        u32::from_le_bytes(data[entry_off + 8..entry_off + 12].try_into().ok()?) as usize;
    let value_off = u32::from_le_bytes(data[entry_off + 12..entry_off + 16].try_into().ok()?);
    if name_off >= data.len() {
        return None;
    }
    let name_bytes = &data[name_off..];
    let end = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_bytes.len());
    let name = &data[name_off..name_off + end];
    Some(ParsedExport {
        sym_hash,
        name,
        value_off,
    })
}

/// Read one resource-meta entry by index.
///
/// Returns `None` if the index is out of range or the data is truncated.
pub fn read_res_meta(data: &[u8], index: u32) -> Option<ResMetaEntry> {
    let hdr = decode(data)?;
    if index >= hdr.res_meta_count {
        return None;
    }
    let entries_off = res_meta_offset(data)?;
    let entry_off = entries_off + (index as usize) * RES_META_SIZE as usize;
    Some(ResMetaEntry {
        res_hash: u64::from_le_bytes(data[entry_off..entry_off + 8].try_into().ok()?),
        sharing_class: data[entry_off + 8],
        lock_prim: u32::from_le_bytes(data[entry_off + 12..entry_off + 16].try_into().ok()?),
    })
}

/// Compute the byte offset of the res_meta entries array.
fn res_meta_offset(data: &[u8]) -> Option<usize> {
    let hdr = decode(data)?;
    let export_entries_off = export_entries_offset(data)?;
    let import_entries_off =
        export_entries_off + (hdr.export_count as usize) * EXPORT_ENTRY_SIZE as usize;
    let word_meta_off =
        import_entries_off + (hdr.import_count as usize) * IMPORT_ENTRY_SIZE as usize;
    let word_meta_end = word_meta_off + (hdr.export_count as usize) * WORD_META_SIZE as usize;
    if word_meta_end > data.len() {
        return None;
    }
    Some(word_meta_end)
}

/// Compute the byte offset of the aperture-use entries array (design doc §5.5),
/// or `None` if the data is truncated.
pub fn aperture_use_offset(data: &[u8]) -> Option<usize> {
    let hdr = decode(data)?;
    let res_meta_end = res_meta_offset(data)? + (hdr.res_meta_count as usize) * RES_META_SIZE as usize;
    if res_meta_end + (hdr.aperture_count as usize) * APERTURE_USE_ENTRY_SIZE as usize > data.len() {
        return None;
    }
    Some(res_meta_end)
}

/// Read one aperture-use entry by index.
///
/// Returns `None` if the index is out of range or the data is truncated.
pub fn read_aperture_use(data: &[u8], index: u32) -> Option<ApertureUseEntry> {
    let hdr = decode(data)?;
    if index >= hdr.aperture_count {
        return None;
    }
    let entries_off = aperture_use_offset(data)?;
    let entry_off = entries_off + (index as usize) * APERTURE_USE_ENTRY_SIZE as usize;
    let name_hash = u64::from_le_bytes(data[entry_off..entry_off + 8].try_into().ok()?);
    let size = u32::from_le_bytes(data[entry_off + 8..entry_off + 12].try_into().ok()?);
    let aperture_id = u16::from_le_bytes(data[entry_off + 12..entry_off + 14].try_into().ok()?);
    Some(ApertureUseEntry {
        name_hash,
        size,
        aperture_id,
        access_mask: data[entry_off + 14],
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::fnv1a_u64;

    fn test_abi_hash() -> u64 {
        crate::abi_hash::compute_abi_hash(1, 8, 64, 2)
    }

    #[test]
    fn encode_roundtrip_minimal() {
        let mut buf = [0u8; 256];
        let ah = test_abi_hash();
        let n = encode_into(&mut buf, b"TestMod", &[], &[], ah, 0, &[], 0, &[]).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.module_name, b"TestMod");
        assert_eq!(decoded.export_count, 0);
        assert_eq!(decoded.import_count, 0);
        assert_eq!(decoded.abi_hash, ah);
        assert_eq!(decoded.modinfo_ver, MODINFO_VER);
        assert_eq!(decoded.aperture_count, 0);
        assert_eq!(decoded.platform_hash, 0);
    }

    #[test]
    fn encode_roundtrip_single_export() {
        let exports = [ExportEntry {
            sym_hash: fnv1a_u64(b"foo"),
            name: b"foo",
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        }];
        let imports = [ImportEntry {
            sym_hash: fnv1a_u64(b"bar"),
            name: b"bar",
        }];
        let mut buf = [0u8; 512];
        let ah = test_abi_hash();
        let n = encode_into(&mut buf, b"M", &exports, &imports, ah, 0, &[], 0, &[]).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.module_name, b"M");
        assert_eq!(decoded.export_count, 1);
        assert_eq!(decoded.import_count, 1);
        assert_eq!(decoded.abi_hash, ah);
    }

    #[test]
    fn encode_small_buffer_returns_none() {
        let exports = [ExportEntry {
            sym_hash: fnv1a_u64(b"x"),
            name: b"x",
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        }];
        let mut buf = [0u8; 16];
        assert!(encode_into(&mut buf, b"X", &exports, &[], 42, 0, &[], 0, &[]).is_none());
    }

    #[test]
    fn encode_header_fields_are_little_endian() {
        let mut buf = [0u8; 256];
        let ah = test_abi_hash();
        let n = encode_into(&mut buf, b"Abc", &[], &[], ah, 0, &[], 0, &[]).unwrap();
        let data = &buf[..n];
        assert_eq!(data[0..4], [0x44, 0x4f, 0x4d, 0x4c]); // "LMOD" LE
        assert_eq!(data[4..6], [4, 0]); // version (v4)
        assert_eq!(data[6..8], [0, 0]); // flags
                                        // abi_hash lives at [8..16]; skip byte-checking it (varies).
        assert_eq!(data[16..20], [48, 0, 0, 0]); // name_off = 48 (header size)
        assert_eq!(data[20..24], [3, 0, 0, 0]); // name_len = 3
        assert_eq!(data[24..28], [0, 0, 0, 0]); // export_count = 0
        assert_eq!(data[28..32], [0, 0, 0, 0]); // import_count = 0
        assert_eq!(data[32..34], [0, 0]); // aperture_count = 0
        assert_eq!(data[34..36], [0, 0]); // res_meta_count = 0
        assert_eq!(data[36..44], [0u8; 8]); // platform_hash = 0
    }

    #[test]
    fn encode_aperture_use_table_roundtrip() {
        let apertures = [
            ApertureUseEntry {
                name_hash: 0x1111_2222_3333_4444,
                size: 0x1000,
                aperture_id: 0,
                access_mask: ACCESS_READ | ACCESS_WRITE | ACCESS_W1C,
            },
            ApertureUseEntry {
                name_hash: 0xaaaa_bbbb_cccc_dddd,
                size: 0x10000,
                aperture_id: 2,
                access_mask: ACCESS_READ | ACCESS_EFFECTFUL_READ,
            },
        ];
        let mut buf = [0u8; 512];
        let ah = test_abi_hash();
        let n = encode_into(&mut buf, b"W", &[], &[], ah, 0, &[], 0xdead_beef_cafe_f00du64, &apertures)
            .unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.aperture_count, 2);
        assert_eq!(decoded.platform_hash, 0xdead_beef_cafe_f00du64);
        for (i, expect) in apertures.iter().enumerate() {
            let got = read_aperture_use(&buf[..n], i as u32).unwrap();
            assert_eq!(got, *expect, "aperture-use entry {i} must survive roundtrip");
        }
        assert!(read_aperture_use(&buf[..n], 2).is_none(), "index past count must be None");
    }

    #[test]
    fn encode_aperture_use_more_than_8_rejected() {
        let apertures = [ApertureUseEntry {
            name_hash: 1,
            size: 0x1000,
            aperture_id: 0,
            access_mask: ACCESS_READ,
        }; 9];
        let mut buf = [0u8; 2048];
        let result = encode_into(&mut buf, b"M", &[], &[], 0, 0, &[], 0, &apertures);
        assert!(result.is_none(), ">8 apertures must return None, not panic");
    }

    #[test]
    fn encode_res_meta_and_apertures_coexist() {
        let res = [ResMetaEntry {
            res_hash: 7,
            sharing_class: 1,
            lock_prim: 2,
        }];
        let apertures = [ApertureUseEntry {
            name_hash: 9,
            size: 0x400,
            aperture_id: 1,
            access_mask: ACCESS_WRITE,
        }];
        let mut buf = [0u8; 512];
        let n = encode_into(&mut buf, b"R", &[], &[], 42, 0, &res, 0x1234, &apertures).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.res_meta_count, 1);
        assert_eq!(decoded.aperture_count, 1);
        assert_eq!(read_res_meta(&buf[..n], 0).unwrap(), res[0]);
        assert_eq!(read_aperture_use(&buf[..n], 0).unwrap(), apertures[0]);
        // The aperture-use table must sit immediately after the res-meta entries.
        let res_end = aperture_use_offset(&buf[..n]).unwrap();
        let res_start = res_end - RES_META_SIZE as usize;
        assert_eq!(read_res_meta(&buf[..n], 0).unwrap().res_hash, 7);
        assert!(res_start >= 48, "res_meta must start after the v4 header");
    }

    #[test]
    fn encode_multi_export_multi_import() {
        let exports = [
            ExportEntry {
                sym_hash: fnv1a_u64(b"add"),
                name: b"add",
                effects: 0,
                requires_caps: 0,
                stack_bound: 0,
            },
            ExportEntry {
                sym_hash: fnv1a_u64(b"sub"),
                name: b"sub",
                effects: 2,
                requires_caps: 1,
                stack_bound: 42,
            },
        ];
        let imports = [ImportEntry {
            sym_hash: fnv1a_u64(b"platform.task.yield"),
            name: b"platform.task.yield",
        }];
        let mut buf = [0u8; 1024];
        let ah = test_abi_hash();
        let n = encode_into(&mut buf, b"Calc", &exports, &imports, ah, 0, &[], 0, &[]).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.module_name, b"Calc");
        assert_eq!(decoded.export_count, 2);
        assert_eq!(decoded.import_count, 1);
        assert_eq!(decoded.abi_hash, ah);
    }

    #[test]
    fn encode_deterministic() {
        let e = [ExportEntry {
            sym_hash: fnv1a_u64(b"f"),
            name: b"f",
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        }];
        let mut buf_a = [0u8; 512];
        let mut buf_b = [0u8; 512];
        let ah = test_abi_hash();
        let n_a = encode_into(&mut buf_a, b"M", &e, &[], ah, 0, &[], 0, &[]).unwrap();
        let n_b = encode_into(&mut buf_b, b"M", &e, &[], ah, 0, &[], 0, &[]).unwrap();
        assert_eq!(n_a, n_b);
        assert_eq!(&buf_a[..n_a], &buf_b[..n_b]);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        assert!(decode(b"\x00\x00\x00\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00").is_none());
    }

    #[test]
    fn decode_rejects_short_data() {
        assert!(decode(b"").is_none());
        assert!(decode(b"LMOD").is_none());
    }

    #[test]
    fn decode_retrieves_abi_hash() {
        let mut buf = [0u8; 256];
        let expected = 0xdeadbeefcafebabeu64;
        let n = encode_into(&mut buf, b"X", &[], &[], expected, 0, &[], 0, &[]).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.abi_hash, expected);
    }

    #[test]
    fn encode_more_than_64_exports_rejected() {
        // The encoder uses fixed-size arrays [u32; 64] for name offsets.
        // Exporting 65 words must return None rather than panicking.
        let e = ExportEntry {
            sym_hash: fnv1a_u64(b"dummy"),
            name: b"dummy",
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        };
        let exports = [e; 65];
        let mut buf = [0u8; 4096];
        let result = encode_into(&mut buf, b"M", &exports, &[], 42, 0, &[], 0, &[]);
        assert!(result.is_none(), ">64 exports must return None, not panic");
    }

    #[test]
    fn encode_more_than_64_imports_rejected() {
        let imp = ImportEntry {
            sym_hash: fnv1a_u64(b"dummy"),
            name: b"dummy",
        };
        let imports = [imp; 65];
        let mut buf = [0u8; 4096];
        let result = encode_into(&mut buf, b"M", &[], &imports, 42, 0, &[], 0, &[]);
        assert!(result.is_none(), ">64 imports must return None, not panic");
    }

    #[test]
    fn word_meta_effect_bits_roundtrip() {
        // Encode a minimal module with one export carrying known effects bits.
        // The effects field lives in the WORD META section, after both
        // export entries and import entries.
        let mut buf = [0u8; 1024];
        let export = ExportEntry {
            sym_hash: fnv1a_u64(b"test"),
            name: b"test",
            effects: 0b0000_0101u16, // bits 0 and 2 set
            requires_caps: 0,
            stack_bound: 0,
        };
        let exports = [export];
        let encoded_len =
            encode_into(&mut buf, b"M", &exports, &[], 0, 0, &[], 0, &[]).expect("encode minimal module");

        // Word meta starts after header + module name strings +
        // 1 export entry (16 bytes) + 0 import entries.
        let hdr = decode(&buf[..encoded_len]).unwrap();
        let export_off = export_entries_offset(&buf[..encoded_len]).unwrap();
        let word_meta_off = export_off
            + hdr.export_count as usize * EXPORT_ENTRY_SIZE as usize
            + hdr.import_count as usize * IMPORT_ENTRY_SIZE as usize;

        // effects at offset 8 within the 16-byte word_meta entry.
        let effects = u16::from_le_bytes(
            buf[word_meta_off + 8..word_meta_off + 10]
                .try_into()
                .unwrap(),
        );
        assert_eq!(effects, 0b0000_0101u16, "effects must survive round-trip");

        // Verify a different-effects module produces different bytes.
        let export2 = ExportEntry {
            sym_hash: fnv1a_u64(b"test"),
            name: b"test",
            effects: 0,
            requires_caps: 0,
            stack_bound: 0,
        };
        let mut buf2 = [0u8; 1024];
        let len2 = encode_into(&mut buf2, b"M", &[export2], &[], 0, 0, &[], 0, &[])
            .expect("encode zero-effects module");
        let hdr2 = decode(&buf2[..len2]).unwrap();
        let export_off2 = export_entries_offset(&buf2[..len2]).unwrap();
        let word_meta_off2 = export_off2
            + hdr2.export_count as usize * EXPORT_ENTRY_SIZE as usize
            + hdr2.import_count as usize * IMPORT_ENTRY_SIZE as usize;
        let effects2 = u16::from_le_bytes(
            buf2[word_meta_off2 + 8..word_meta_off2 + 10]
                .try_into()
                .unwrap(),
        );
        assert_eq!(effects2, 0, "zero-effects export must decode to 0");
        assert_ne!(
            &buf[..encoded_len],
            &buf2[..len2],
            "differing effects must produce differing modinfo bytes"
        );
    }
}
