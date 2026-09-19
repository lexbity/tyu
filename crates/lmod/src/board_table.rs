//! P6 board aperture table codec (design doc §5.8, decisions D-4/D-5).
//!
//! The device-side projection of a platform descriptor: exactly what the
//! on-device loader needs to bind a module — `platform_hash`, and each aperture's
//! stable `name_hash` (FNV-1a-64 of the board aperture name), `base`, `size`, and
//! the board-declared `capability` (fused register access bits).
//!
//! The blob is produced at build time by `tyu` (which computes the capability
//! from the compiled descriptor) and embedded in the firmware via linker
//! symbols; the on-device loader decodes it with this *same* codec — never TOML
//! on the device, never a second implementation (NFR-6). The codec lives in
//! `lmod` (not `codegen-core`) so the `no_std` loader gains no `frontend`/`ir`
//! footprint: it is pure fixed-struct byte parsing.

use crate::hash::fnv1a_u64;

/// Maximum apertures a board table may carry (matches the module/loader cap).
pub const APERTURE_CAP: usize = 8;
/// Wire size of one board-aperture entry in the serialized table.
pub const BOARD_TABLE_ENTRY_SIZE: usize = 24;
/// Fixed serialized size bound of the board table (16 B header + 8 entries).
pub const BOARD_TABLE_ENCODED_SIZE: usize = 16 + APERTURE_CAP * BOARD_TABLE_ENTRY_SIZE;

/// A aperture the board exposes, as the loader sees it (design doc §5.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardAperture {
    /// FNV-1a-64 of the board aperture's name — the stable identity a module's
    /// aperture-use entries match against (E5222).
    pub name_hash: u64,
    /// The bound base address the loader writes into reloc sites (E5220-guarded).
    pub base: u32,
    /// Aperture size; a module whose `size` disagrees is rejected (E5222).
    pub size: u32,
    /// Fused access bits the board declares for this aperture (union over its
    /// devices' registers: `ACCESS_READ|WRITE|W1S|W1C|EFFECTFUL_READ`).
    pub capability: u8,
}

/// The decoded board table carried by the loader platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardTable {
    pub platform_hash: u64,
    pub apertures: [BoardAperture; APERTURE_CAP],
    pub aperture_count: usize,
}

impl BoardTable {
    pub const fn empty() -> Self {
        Self {
            platform_hash: 0,
            apertures: [BoardAperture {
                name_hash: 0,
                base: 0,
                size: 0,
                capability: 0,
            }; APERTURE_CAP],
            aperture_count: 0,
        }
    }

    pub fn apertures(&self) -> &[BoardAperture] {
        &self.apertures[..self.aperture_count]
    }
}

impl Default for BoardTable {
    fn default() -> Self {
        Self::empty()
    }
}

/// Errors from board-table encode/decode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoardTableError {
    BufferTooSmall,
    Truncated,
    TrailingBytes,
    TooManyApertures,
    BadEntry,
}

impl BoardTableError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BufferTooSmall => "board table output buffer too small",
            Self::Truncated => "board table truncated",
            Self::TrailingBytes => "trailing bytes after board table",
            Self::TooManyApertures => "board table carries more than 8 apertures",
            Self::BadEntry => "malformed board table entry",
        }
    }
}

impl core::fmt::Display for BoardTableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Compute a aperture's stable `name_hash` (the identity the loader matches).
pub fn aperture_name_hash(name: &[u8]) -> u64 {
    fnv1a_u64(name)
}

/// Encode a board table into `out`, returning the number of bytes written.
///
/// Wire layout (little-endian):
/// ```text
/// 0  8  platform_hash
/// 8  1  aperture_count
/// 9  7  _pad
/// 16 .. per aperture (24 B): u64 name_hash · u32 base · u32 size
///                          · u8 capability · u8[7] _pad
/// ```
pub fn encode_into(
    out: &mut [u8],
    platform_hash: u64,
    apertures: &[BoardAperture],
) -> Result<usize, BoardTableError> {
    if apertures.len() > APERTURE_CAP {
        return Err(BoardTableError::TooManyApertures);
    }
    let total = 16 + apertures.len() * BOARD_TABLE_ENTRY_SIZE;
    if out.len() < total {
        return Err(BoardTableError::BufferTooSmall);
    }
    out[..total].fill(0);
    out[0..8].copy_from_slice(&platform_hash.to_le_bytes());
    out[8] = apertures.len() as u8;
    for (i, w) in apertures.iter().enumerate() {
        let off = 16 + i * BOARD_TABLE_ENTRY_SIZE;
        out[off..off + 8].copy_from_slice(&w.name_hash.to_le_bytes());
        out[off + 8..off + 12].copy_from_slice(&w.base.to_le_bytes());
        out[off + 12..off + 16].copy_from_slice(&w.size.to_le_bytes());
        out[off + 16] = w.capability;
    }
    Ok(total)
}

/// Decode a board table blob. Strict: rejects trailing bytes, counts past the
/// cap, and truncated entries (panic-free, `no_std`).
pub fn decode(bytes: &[u8]) -> Result<BoardTable, BoardTableError> {
    if bytes.len() > BOARD_TABLE_ENCODED_SIZE {
        return Err(BoardTableError::TrailingBytes);
    }
    if bytes.len() < 16 {
        return Err(BoardTableError::Truncated);
    }
    let platform_hash = u64::from_le_bytes(bytes[0..8].try_into().map_err(|_| BoardTableError::Truncated)?);
    let count = bytes[8] as usize;
    if count > APERTURE_CAP {
        return Err(BoardTableError::TooManyApertures);
    }
    let expected = 16 + count * BOARD_TABLE_ENTRY_SIZE;
    if bytes.len() < expected {
        return Err(BoardTableError::Truncated);
    }
    if bytes.len() != expected {
        return Err(BoardTableError::TrailingBytes);
    }
    let mut apertures = [BoardAperture {
        name_hash: 0,
        base: 0,
        size: 0,
        capability: 0,
    }; APERTURE_CAP];
    for (i, slot) in apertures.iter_mut().take(count).enumerate() {
        let off = 16 + i * BOARD_TABLE_ENTRY_SIZE;
        let name_hash = u64::from_le_bytes(
            bytes[off..off + 8].try_into().map_err(|_| BoardTableError::BadEntry)?,
        );
        let base =
            u32::from_le_bytes(bytes[off + 8..off + 12].try_into().map_err(|_| BoardTableError::BadEntry)?);
        let size =
            u32::from_le_bytes(bytes[off + 12..off + 16].try_into().map_err(|_| BoardTableError::BadEntry)?);
        let capability = bytes[off + 16];
        *slot = BoardAperture {
            name_hash,
            base,
            size,
            capability,
        };
    }
    Ok(BoardTable {
        platform_hash,
        apertures,
        aperture_count: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn sample() -> Vec<BoardAperture> {
        vec![
            BoardAperture {
                name_hash: fnv1a_u64(b"apb"),
                base: 0x4000_0000,
                size: 0x1_0000,
                capability: 0b11,
            },
            BoardAperture {
                name_hash: fnv1a_u64(b"scratch"),
                base: 0x2000_0000,
                size: 0x1_000,
                capability: 0b10011,
            },
        ]
    }

    #[test]
    fn encode_decode_roundtrip() {
        let apertures = sample();
        let mut buf = [0u8; BOARD_TABLE_ENCODED_SIZE];
        let n = encode_into(&mut buf, 0x1234_5678_9abc_def0, &apertures).unwrap();
        let decoded = decode(&buf[..n]).unwrap();
        assert_eq!(decoded.platform_hash, 0x1234_5678_9abc_def0);
        assert_eq!(decoded.aperture_count, 2);
        assert_eq!(decoded.apertures[0], apertures[0]);
        assert_eq!(decoded.apertures[1], apertures[1]);
    }

    #[test]
    fn name_hash_is_stable() {
        assert_eq!(aperture_name_hash(b"apb"), fnv1a_u64(b"apb"));
    }

    #[test]
    fn decode_rejects_truncated_and_trailing() {
        let mut buf = [0u8; BOARD_TABLE_ENCODED_SIZE];
        let n = encode_into(&mut buf, 0, &sample()).unwrap();
        assert_eq!(decode(&buf[..15]).unwrap_err(), BoardTableError::Truncated);
        assert_eq!(decode(&buf[..20]).unwrap_err(), BoardTableError::Truncated);
        let mut extended = [0u8; BOARD_TABLE_ENCODED_SIZE + 1];
        extended[..n].copy_from_slice(&buf[..n]);
        assert_eq!(decode(&extended).unwrap_err(), BoardTableError::TrailingBytes);
    }

    #[test]
    fn encode_rejects_too_many_apertures() {
        let apertures = vec![
            BoardAperture {
                name_hash: 0,
                base: 0,
                size: 0x100,
                capability: 0,
            };
            APERTURE_CAP + 1
        ];
        let mut buf = [0u8; BOARD_TABLE_ENCODED_SIZE];
        assert_eq!(
            encode_into(&mut buf, 0, &apertures).unwrap_err(),
            BoardTableError::TooManyApertures
        );
    }

    #[test]
    fn encode_is_deterministic() {
        let mut a = [0u8; BOARD_TABLE_ENCODED_SIZE];
        let mut b = [0u8; BOARD_TABLE_ENCODED_SIZE];
        let na = encode_into(&mut a, 0xabc, &sample()).unwrap();
        let nb = encode_into(&mut b, 0xabc, &sample()).unwrap();
        assert_eq!(na, nb);
        assert_eq!(&a[..na], &b[..nb]);
    }
}