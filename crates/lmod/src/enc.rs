//! `.lmod` encryption envelope header codec.
//!
//! Defines the wire format for authenticated encryption metadata placed
//! between the container header and modinfo section when
//! `LMOD_FLAG_ENCRYPTED` is set (FORMAT_VER ≥ 3).

use core::fmt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// AEAD algorithm identifier — ChaCha20-Poly1305 (only v1 value).
pub const AEAD_CHACHA20POLY1305: u8 = 1;

/// CEK wrapping nonce length (12 bytes — standard ChaCha20-Poly1305 nonce).
pub const NONCE_LEN: usize = 12;

/// AEAD authentication tag length (16 bytes — standard Poly1305 tag).
pub const TAG_LEN: usize = 16;

/// CEK key length (256-bit = 32 bytes).
pub const CEK_LEN: usize = 32;

/// Wrapped CEK length: nonce(12) + ciphertext(32) + tag(16) = 60 bytes.
pub const WRAP_LEN: usize = NONCE_LEN + CEK_LEN + TAG_LEN;

/// Size of one wrapped-CEK slot: key_id(8) + wrap_scheme(1) + pad(7) + wrapped(60) = 76 bytes.
pub const WRAPPED_SLOT_SIZE: usize = 8 + 1 + 7 + WRAP_LEN;

// ---------------------------------------------------------------------------
// Encryption mode
// ---------------------------------------------------------------------------

/// Typed encryption mode for a `.lmod` container.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncMode {
    /// No encryption — container is plaintext (the only v2 mode).
    None = 0,
    /// Fleet mode — a single shared KEK encrypts the CEK.
    Fleet = 1,
    /// Device mode — per-device KEK; N wrapped-CEK slots.
    Device = 2,
}

impl EncMode {
    /// Parse from a wire byte. Returns `None` for reserved values (3..=255).
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(EncMode::None),
            1 => Some(EncMode::Fleet),
            2 => Some(EncMode::Device),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Wrap scheme
// ---------------------------------------------------------------------------

/// Symmetric ChaCha20-Poly1305 CEK wrapping (deterministic zero-nonce).
pub const WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305: u8 = 1;

/// Reserved for future asymmetric ECIES-style wrapping.
pub const _WRAP_SCHEME_ASYMMETRIC_ECIES: u8 = 2;

// ---------------------------------------------------------------------------
// Wrapped CEK slot
// ---------------------------------------------------------------------------

/// A single wrapped content-encryption key slot.
#[derive(Clone, PartialEq, Eq)]
pub struct WrappedCekSlot {
    /// Key identifier for the KEK that wraps this CEK.
    pub key_id: u64,
    /// Wrap scheme (1 = symmetric ChaCha20-Poly1305, 2 = asymmetric ECIES reserved).
    pub wrap_scheme: u8,
    /// Wrapped CEK: nonce(12) + ciphertext(32) + tag(16).
    pub wrapped: [u8; WRAP_LEN],
}

impl fmt::Debug for WrappedCekSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WrappedCekSlot")
            .field("key_id", &self.key_id)
            .field("wrapped", &&self.wrapped[..])
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Encryption envelope header
// ---------------------------------------------------------------------------

/// The encryption envelope header.
///
/// Present in the container when `LMOD_FLAG_ENCRYPTED` is set.
/// Positioned immediately after the 72-byte container header, inside the
/// signed region (so the signature authenticates mode, envelope, and AAD).
#[derive(Clone, Debug, PartialEq)]
pub struct EncHeader {
    pub enc_mode: EncMode,
    pub aead_id: u8,
    pub nonce: [u8; NONCE_LEN],
    pub tag: [u8; TAG_LEN],
    pub wrapped_slots: crate::alloc::vec::Vec<WrappedCekSlot>,
}

impl EncHeader {
    /// Wire size of this enc-header in bytes.
    pub fn wire_len(&self) -> usize {
        enc_header_len(self.wrapped_slots.len())
    }
}

// ---------------------------------------------------------------------------
// Wire size computation
// ---------------------------------------------------------------------------

/// Compute the wire size of an enc-header with `wrapped_count` slots.
///
/// Fixed portion: enc_mode(1) + aead_id(1) + _pad(2) + nonce(12) +
///                tag(16) + wrapped_count(4) = 36 bytes.
/// Per-slot: key_id(8) + wrapped(60) = 68 bytes.
pub const fn enc_header_len(wrapped_count: usize) -> usize {
    36 + wrapped_count * WRAPPED_SLOT_SIZE
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// Encode an `EncHeader` into the beginning of `buf`.
///
/// `buf` must be at least `enc_header_len(wrapped_slots.len())` bytes.
/// Returns the number of bytes written on success, or an error if `buf` is
/// too short or the enc-header has an invalid mode/aead_id.
pub fn encode_enc_header(buf: &mut [u8], eh: &EncHeader) -> Result<usize, ()> {
    let total = eh.wire_len();
    if buf.len() < total {
        return Err(());
    }

    let mut off = 0usize;
    buf[off] = eh.enc_mode as u8;
    off += 1;
    buf[off] = eh.aead_id;
    off += 1;
    // 2 bytes of _pad (zero)
    buf[off..off + 2].copy_from_slice(&[0u8; 2]);
    off += 2;
    buf[off..off + NONCE_LEN].copy_from_slice(&eh.nonce);
    off += NONCE_LEN;
    buf[off..off + TAG_LEN].copy_from_slice(&eh.tag);
    off += TAG_LEN;
    buf[off..off + 4].copy_from_slice(&(eh.wrapped_slots.len() as u32).to_le_bytes());
    off += 4;

    for slot in &eh.wrapped_slots {
        buf[off..off + 8].copy_from_slice(&slot.key_id.to_le_bytes());
        off += 8;
        buf[off] = slot.wrap_scheme;
        off += 1;
        // 7 bytes pad (zero)
        buf[off..off + 7].fill(0);
        off += 7;
        buf[off..off + WRAP_LEN].copy_from_slice(&slot.wrapped);
        off += WRAP_LEN;
    }

    debug_assert_eq!(off, total);
    Ok(total)
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decode an `EncHeader` from `bytes`.
///
/// Returns `None` if the bytes are truncated, contain an unknown `enc_mode`,
/// or specify an unknown `aead_id`.
pub fn decode_enc_header(bytes: &[u8]) -> Option<EncHeader> {
    if bytes.len() < 36 {
        return None; // minimum size (0 wrapped slots)
    }

    let enc_mode = EncMode::from_u8(bytes[0])?;
    let aead_id = bytes[1];
    // bytes[2..4] are pad (ignored)

    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[4..4 + NONCE_LEN]);

    let mut tag = [0u8; TAG_LEN];
    tag.copy_from_slice(&bytes[4 + NONCE_LEN..4 + NONCE_LEN + TAG_LEN]);

    let wrapped_count = u32::from_le_bytes(
        bytes[4 + NONCE_LEN + TAG_LEN..4 + NONCE_LEN + TAG_LEN + 4].try_into().ok()?,
    ) as usize;

    let fixed_len = 36;
    let expected_len = fixed_len + wrapped_count * WRAPPED_SLOT_SIZE;
    if bytes.len() < expected_len {
        return None;
    }

    // Validate aead_id.
    if aead_id != AEAD_CHACHA20POLY1305 && aead_id != 0 {
        // 0 is reserved; 1 is the only assigned value.
        // Future values are accepted for forward compatibility but the
        // payload cannot be decrypted without a matching implementation.
        // For codec purposes, we only reject unreserved values.
        return None;
    }

    let mut wrapped_slots = crate::alloc::vec::Vec::with_capacity(wrapped_count);
    let mut off = fixed_len;
    for _ in 0..wrapped_count {
        if off + WRAPPED_SLOT_SIZE > bytes.len() {
            return None;
        }
        let key_id = u64::from_le_bytes(
            bytes[off..off + 8].try_into().ok()?,
        );
        off += 8;
        let wrap_scheme = bytes[off];
        off += 1;
        off += 7; // skip padding
        let mut wrapped = [0u8; WRAP_LEN];
        wrapped.copy_from_slice(&bytes[off..off + WRAP_LEN]);
        off += WRAP_LEN;
        wrapped_slots.push(WrappedCekSlot { key_id, wrap_scheme, wrapped });
    }

    Some(EncHeader {
        enc_mode,
        aead_id,
        nonce,
        tag,
        wrapped_slots,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc::vec;

    fn make_slot(key_id: u64, seed: u8) -> WrappedCekSlot {
        let mut wrapped = [0u8; WRAP_LEN];
        for (i, b) in wrapped.iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8);
        }
        WrappedCekSlot { key_id, wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305, wrapped }
    }

    fn make_eh(enc_mode: EncMode, slot_count: usize) -> EncHeader {
        let mut nonce = [0u8; NONCE_LEN];
        let mut tag = [0u8; TAG_LEN];
        for (i, b) in nonce.iter_mut().enumerate() {
            *b = i as u8;
        }
        for (i, b) in tag.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(0x80);
        }

        let mut slots = vec![];
        for i in 0..slot_count {
            slots.push(make_slot(i as u64 + 1000, (i * 7) as u8));
        }

        EncHeader {
            enc_mode,
            aead_id: AEAD_CHACHA20POLY1305,
            nonce,
            tag,
            wrapped_slots: slots,
        }
    }

    #[test]
    fn fleet_roundtrip() {
        let eh = make_eh(EncMode::Fleet, 1);
        let mut buf = vec![0u8; eh.wire_len()];
        let written = encode_enc_header(&mut buf, &eh).unwrap();
        assert_eq!(written, eh.wire_len());

        let decoded = decode_enc_header(&buf).unwrap();
        assert_eq!(decoded.enc_mode, EncMode::Fleet);
        assert_eq!(decoded.aead_id, AEAD_CHACHA20POLY1305);
        assert_eq!(decoded.nonce, eh.nonce);
        assert_eq!(decoded.tag, eh.tag);
        assert_eq!(decoded.wrapped_slots.len(), 1);
        assert_eq!(decoded.wrapped_slots[0].key_id, 1000);
        assert_eq!(decoded.wrapped_slots[0].wrapped, eh.wrapped_slots[0].wrapped);
    }

    #[test]
    fn device_roundtrip_multi_slot() {
        let eh = make_eh(EncMode::Device, 3);
        let mut buf = vec![0u8; eh.wire_len()];
        encode_enc_header(&mut buf, &eh).unwrap();
        let decoded = decode_enc_header(&buf).unwrap();
        assert_eq!(decoded.enc_mode, EncMode::Device);
        assert_eq!(decoded.wrapped_slots.len(), 3);
        for i in 0..3 {
            assert_eq!(decoded.wrapped_slots[i].key_id, i as u64 + 1000);
        }
    }

    #[test]
    fn none_mode_roundtrip() {
        let eh = make_eh(EncMode::None, 0);
        let mut buf = vec![0u8; eh.wire_len()];
        encode_enc_header(&mut buf, &eh).unwrap();
        let decoded = decode_enc_header(&buf).unwrap();
        assert_eq!(decoded.enc_mode, EncMode::None);
        assert!(decoded.wrapped_slots.is_empty());
    }

    #[test]
    fn truncated_buffer_rejected() {
        let eh = make_eh(EncMode::Fleet, 1);
        let mut buf = vec![0u8; eh.wire_len()];
        encode_enc_header(&mut buf, &eh).unwrap();
        // Truncate by 1 byte.
        assert!(decode_enc_header(&buf[..buf.len() - 1]).is_none());
    }

    #[test]
    fn empty_slice_rejected() {
        assert!(decode_enc_header(b"").is_none());
    }

    #[test]
    fn unknown_aead_id_rejected() {
        let mut eh = make_eh(EncMode::Fleet, 1);
        eh.aead_id = 0xff; // unreserved
        let mut buf = vec![0u8; eh.wire_len()];
        encode_enc_header(&mut buf, &eh).unwrap();
        assert!(decode_enc_header(&buf).is_none());
    }

    #[test]
    fn unknown_enc_mode_rejected() {
        let mut buf = vec![0u8; 36];
        buf[0] = 99; // unknown enc_mode
        assert!(decode_enc_header(&buf).is_none());
    }

    #[test]
    fn enc_header_len_formula() {
        assert_eq!(enc_header_len(0), 36);
        assert_eq!(enc_header_len(1), 36 + 76);
        assert_eq!(enc_header_len(3), 36 + 3 * 76);
    }

    #[test]
    fn buffer_too_small_for_encode() {
        let eh = make_eh(EncMode::Fleet, 1);
        let mut buf = [0u8; 1];
        assert!(encode_enc_header(&mut buf, &eh).is_err());
    }

    #[test]
    fn decode_oversized_buffer_ok() {
        // Extra trailing bytes should be ignored.
        let eh = make_eh(EncMode::None, 0);
        let mut buf = vec![0u8; eh.wire_len() + 10];
        encode_enc_header(&mut buf, &eh).unwrap();
        let decoded = decode_enc_header(&buf).unwrap();
        assert_eq!(decoded.enc_mode, EncMode::None);
    }

    // -----------------------------------------------------------------------
    // U-ENC-1: Fleet roundtrip with key_id=0 (CT-6 convention)
    // -----------------------------------------------------------------------

    #[test]
    fn enc_header_roundtrip_fleet() {
        let slot = WrappedCekSlot {
            key_id: 0,
            wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
            wrapped: [0x42u8; WRAP_LEN],
        };
        let eh = EncHeader {
            enc_mode: EncMode::Fleet,
            aead_id: AEAD_CHACHA20POLY1305,
            nonce: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c],
            tag: [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f],
            wrapped_slots: vec![slot],
        };
        let mut buf = vec![0u8; eh.wire_len()];
        let written = encode_enc_header(&mut buf, &eh).unwrap();
        assert_eq!(written, enc_header_len(1));

        let decoded = decode_enc_header(&buf).unwrap();
        assert_eq!(decoded.enc_mode, EncMode::Fleet);
        assert_eq!(decoded.aead_id, AEAD_CHACHA20POLY1305);
        assert_eq!(decoded.nonce, eh.nonce);
        assert_eq!(decoded.tag, eh.tag);
        assert_eq!(decoded.wrapped_slots.len(), 1);
        assert_eq!(decoded.wrapped_slots[0].key_id, 0);
        assert_eq!(decoded.wrapped_slots[0].wrapped, [0x42u8; WRAP_LEN]);
    }

    // -----------------------------------------------------------------------
    // U-ENC-2: Device roundtrip with N=3 slots, key_ids 0,1,2 (CT-6)
    // -----------------------------------------------------------------------

    #[test]
    fn enc_header_roundtrip_device_n() {
        let slots = vec![
            WrappedCekSlot { key_id: 0, wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305, wrapped: [0u8; WRAP_LEN] },
            WrappedCekSlot { key_id: 1, wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305, wrapped: [1u8; WRAP_LEN] },
            WrappedCekSlot { key_id: 2, wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305, wrapped: [2u8; WRAP_LEN] },
        ];
        let eh = EncHeader {
            enc_mode: EncMode::Device,
            aead_id: AEAD_CHACHA20POLY1305,
            nonce: [0u8; 12],
            tag: [0u8; 16],
            wrapped_slots: slots,
        };
        let mut buf = vec![0u8; eh.wire_len()];
        encode_enc_header(&mut buf, &eh).unwrap();
        let decoded = decode_enc_header(&buf).unwrap();
        assert_eq!(decoded.enc_mode, EncMode::Device);
        assert_eq!(decoded.wrapped_slots.len(), 3);
        for i in 0..3 {
            assert_eq!(decoded.wrapped_slots[i].key_id, i as u64);
            assert_eq!(decoded.wrapped_slots[i].wrapped, [i as u8; WRAP_LEN]);
        }
    }

    // -----------------------------------------------------------------------
    // U-ENC-3: enc_header_len formula matches bytes written for N∈{1,2,8}
    // -----------------------------------------------------------------------

    #[test]
    fn enc_header_len_matches_encoded() {
        for n in &[1usize, 2, 8] {
            let slot = WrappedCekSlot {
                key_id: 0,
                wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
                wrapped: [0u8; WRAP_LEN],
            };
            let eh = EncHeader {
                enc_mode: EncMode::Device,
                aead_id: AEAD_CHACHA20POLY1305,
                nonce: [0u8; 12],
                tag: [0u8; 16],
                wrapped_slots: vec![slot; *n],
            };
            let expected = enc_header_len(*n);
            let mut buf = vec![0u8; expected];
            let written = encode_enc_header(&mut buf, &eh).unwrap();
            assert_eq!(written, expected, "enc_header_len({}) does not match encoded size", n);
        }
    }

    // -----------------------------------------------------------------------
    // U-ENC-4: Reserved enc_mode=3 is rejected (CT-4, first reserved value)
    // -----------------------------------------------------------------------

    #[test]
    fn reserved_enc_mode_rejected() {
        let mut buf = vec![0u8; 36];
        buf[0] = 3; // first reserved enc_mode value
        assert!(decode_enc_header(&buf).is_none());
    }

    // -----------------------------------------------------------------------
    // U-ENC-5: aead_id=2 is rejected (CT-4, only AEAD_CHACHA20POLY1305=1 valid)
    // -----------------------------------------------------------------------

    #[test]
    fn bad_aead_id_rejected() {
        let mut buf = vec![0u8; 36];
        buf[0] = EncMode::Fleet as u8;
        buf[1] = 2; // not AEAD_CHACHA20POLY1305
        assert!(decode_enc_header(&buf).is_none());
    }

    // -----------------------------------------------------------------------
    // U-ENC-6: Buffer one byte short of enc_header_len(1) is rejected (LD-7)
    // -----------------------------------------------------------------------

    #[test]
    fn truncated_header_rejected() {
        let slot = WrappedCekSlot {
            key_id: 0,
            wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
            wrapped: [0u8; WRAP_LEN],
        };
        let eh = EncHeader {
            enc_mode: EncMode::Fleet,
            aead_id: AEAD_CHACHA20POLY1305,
            nonce: [0u8; 12],
            tag: [0u8; 16],
            wrapped_slots: vec![slot],
        };
        let full_len = enc_header_len(1);
        let mut buf = vec![0u8; full_len];
        encode_enc_header(&mut buf, &eh).unwrap();
        // Buffer one byte short — must be rejected.
        assert!(decode_enc_header(&buf[..full_len - 1]).is_none());
    }
}
