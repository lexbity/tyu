//! `.lang.modinfo` wire-format codec.
//!
//! Canonical definition: abi-contract.md §3–§4.
//! Companion container layout: module-format-and-loading.md §3–§4.

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const LMOD_MAGIC: u32 = 0x4c4d4f44; // "LMOD"
pub const MODINFO_VER: u16 = 2;

pub const MODINFO_HEADER_SIZE: u32 = 32;

pub const EXPORT_ENTRY_SIZE: u32 = 16; // u64 sym_hash + u32 name_off + u32 value_off
pub const IMPORT_ENTRY_SIZE: u32 = 12; // u64 sym_hash + u32 name_off
pub const WORD_META_SIZE: u32 = 16;    // u64 sym_hash + u16 effects + u16 requires_caps + u32 stack_bound
pub const RES_META_SIZE: u32 = 16;     // u64 res_hash + u8 sharing_class + u8[3] _pad + u32 lock_prim

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
) -> Option<usize> {
    let export_count = exports.len() as u32;
    let import_count = imports.len() as u32;

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

    if (total as usize) > buf.len() {
        return None;
    }

    // Clear the used portion so padding bytes are zero.
    for b in buf[..total as usize].iter_mut() {
        *b = 0;
    }

    let mut off: usize = 0;

    // 1. Fixed header (32 bytes)
    poke_u32(buf, off, LMOD_MAGIC); off += 4;
    poke_u16(buf, off, MODINFO_VER); off += 2;
    poke_u16(buf, off, 0);                   off += 2; // flags
    poke_u64(buf, off, abi_hash_val);        off += 8; // abi_hash (abi-contract §5)
    let name_off_pos = off; poke_u32(buf, off, 0); off += 4; // placeholder
    let name_len_pos = off; poke_u32(buf, off, 0); off += 4; // placeholder
    poke_u32(buf, off, export_count);        off += 4;
    poke_u32(buf, off, import_count);        off += 4;
    // header = 32 bytes

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
    let word_meta_base = export_entries_off
        + export_count * EXPORT_ENTRY_SIZE
        + import_count * IMPORT_ENTRY_SIZE;

    for (i, e) in exports.iter().enumerate() {
        let value_off = word_meta_base + i as u32 * WORD_META_SIZE;
        poke_u64(buf, off, e.sym_hash); off += 8;
        poke_u32(buf, off, export_name_offs[i]); off += 4;
        poke_u32(buf, off, value_off); off += 4;
    }

    // 4. Import entries
    for (i, imp) in imports.iter().enumerate() {
        poke_u64(buf, off, imp.sym_hash); off += 8;
        poke_u32(buf, off, import_name_offs[i]); off += 4;
    }

    // 5. Word meta entries
    for e in exports.iter() {
        poke_u64(buf, off, e.sym_hash); off += 8;
        poke_u16(buf, off, e.effects); off += 2;
        poke_u16(buf, off, e.requires_caps); off += 2;
        poke_u32(buf, off, e.stack_bound); off += 4;
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
}

/// Compute the byte offset of the export entries array within the modinfo
/// data, or `None` if the data is truncated.
///
/// The name table layout (re-encoded from the encoder in this same file):
///   - header (MODINFO_HEADER_SIZE bytes)
///   - module name bytes (name_len, padded to 4)
///   - export names (NUL-terminated each, padded to 4)
///   - import names (NUL-terminated each, padded to 4)
///   - export entries follow immediately
pub fn export_entries_offset(data: &[u8]) -> Option<usize> {
    let hdr = decode(data)?;
    let mut off = MODINFO_HEADER_SIZE as usize;

    // Skip module name
    off += hdr.module_name_len as usize;
    off = (off + 3) & !3;

    // Skip export names
    for _ in 0..hdr.export_count {
        while off < data.len() && data[off] != 0 {
            off += 1;
        }
        if off >= data.len() {
            return None;
        }
        off += 1; // skip NUL
    }
    off = (off + 3) & !3;

    // Skip import names
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
///
/// Returns `None` if the index is out of range or the data is truncated.
/// The `name` field borrows from `data` (a NUL-terminated string in the
/// name table).
pub fn read_export<'a>(data: &'a [u8], index: u32) -> Option<ParsedExport<'a>> {
    let hdr = decode(data)?;
    if index >= hdr.export_count {
        return None;
    }
    let entries_off = export_entries_offset(data)?;
    let entry_off = entries_off + (index as usize) * EXPORT_ENTRY_SIZE as usize;

    let sym_hash = u64::from_le_bytes(data[entry_off..entry_off + 8].try_into().ok()?);
    let name_off = u32::from_le_bytes(data[entry_off + 8..entry_off + 12].try_into().ok()?) as usize;
    let value_off = u32::from_le_bytes(data[entry_off + 12..entry_off + 16].try_into().ok()?);

    if name_off >= data.len() {
        return None;
    }
    let name_bytes = &data[name_off..];
    let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
    let name = &data[name_off..name_off + end];

    Some(ParsedExport {
        sym_hash,
        name,
        value_off,
    })
}

/// A single export entry decoded from raw bytes (name borrows from source).
#[derive(Clone, Debug)]
pub struct ParsedExport<'a> {
    pub sym_hash: u64,
    pub name: &'a [u8],
    pub value_off: u32,
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
    let abi_hash = u64::from_le_bytes(data[8..16].try_into().ok()?);
    let name_off = u32::from_le_bytes(data[16..20].try_into().ok()?) as usize;
    let name_len = u32::from_le_bytes(data[20..24].try_into().ok()?) as usize;
    let export_count = u32::from_le_bytes(data[24..28].try_into().ok()?);
    let import_count = u32::from_le_bytes(data[28..32].try_into().ok()?);

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
        crate::abi_hash::compute_abi_hash(8, 64, 2)
    }

    #[test]
    fn encode_roundtrip_minimal() {
        let mut buf = [0u8; 256];
        let ah = test_abi_hash();
        let n = encode_into(&mut buf, b"TestMod", &[], &[], ah).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.module_name, b"TestMod");
        assert_eq!(decoded.export_count, 0);
        assert_eq!(decoded.import_count, 0);
        assert_eq!(decoded.abi_hash, ah);
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
        let n = encode_into(&mut buf, b"M", &exports, &imports, ah).unwrap();
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
        assert!(encode_into(&mut buf, b"X", &exports, &[], 42).is_none());
    }

    #[test]
    fn encode_header_fields_are_little_endian() {
        let mut buf = [0u8; 256];
        let ah = test_abi_hash();
        let n = encode_into(&mut buf, b"Abc", &[], &[], ah).unwrap();
        let data = &buf[..n];
        assert_eq!(data[0..4], [0x44, 0x4f, 0x4d, 0x4c]); // "LMOD" LE
        assert_eq!(data[4..6], [2, 0]);  // version
        assert_eq!(data[6..8], [0, 0]);  // flags
        // abi_hash lives at [8..16]; skip byte-checking it (varies).
        assert_eq!(data[16..20], [32, 0, 0, 0]); // name_off = 32 (header size)
        assert_eq!(data[20..24], [3, 0, 0, 0]); // name_len = 3
        assert_eq!(data[24..28], [0, 0, 0, 0]); // export_count = 0
        assert_eq!(data[28..32], [0, 0, 0, 0]); // import_count = 0
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
        let n = encode_into(&mut buf, b"Calc", &exports, &imports, ah).unwrap();
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
        let n_a = encode_into(&mut buf_a, b"M", &e, &[], ah).unwrap();
        let n_b = encode_into(&mut buf_b, b"M", &e, &[], ah).unwrap();
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
        let n = encode_into(&mut buf, b"X", &[], &[], expected).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.abi_hash, expected);
    }
}
