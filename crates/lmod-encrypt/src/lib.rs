//! `.lmod` encryption library.
//!
//! Encrypts the payload (code/rodata/data) using ChaCha20-Poly1305 with a
//! random CEK, wrapped under one or more KEKs.

use core::fmt;

use chacha20poly1305::aead::{Aead, AeadInPlace, KeyInit, OsRng, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use lmod::enc::{
    enc_header_len, encode_enc_header, EncHeader, EncMode, WrappedCekSlot,
    AEAD_CHACHA20POLY1305, CEK_LEN, NONCE_LEN, TAG_LEN, WRAP_LEN,
    WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
};
use lmod::header::{encode_header, compute_layout, LMOD_FLAG_ENCRYPTED, HEADER_SIZE, FORMAT_VER};
use lmod::validate::Container;
use rand_core::RngCore;

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

/// Errors that can occur during encryption.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncError {
    /// The input is not a valid `.lmod` container.
    InvalidContainer,
    /// ChaCha20-Poly1305 payload encryption failed.
    PayloadEncryptionFailed,
}

impl fmt::Display for EncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidContainer => write!(f, "invalid .lmod container"),
            Self::PayloadEncryptionFailed => write!(f, "payload encryption failed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Encryption configuration
// ---------------------------------------------------------------------------

/// Encryption configuration for a single KEK (fleet mode).
pub struct FleetConfig {
    /// 32-byte Key-Encryption Key.
    pub kek: [u8; 32],
}

/// Encryption configuration for per-device KEKs (device mode).
pub struct DeviceConfig {
    /// Per-device KEKs: list of (device_id, 32-byte key).
    pub device_keys: Vec<(String, [u8; 32])>,
}

// ---------------------------------------------------------------------------
// Core encryption functions
// ---------------------------------------------------------------------------

/// Wrap a CEK under a KEK using deterministic zero-nonce ChaCha20-Poly1305.
fn wrap_cek(kek: &[u8], cek: &[u8; CEK_LEN], key_id: u64) -> WrappedCekSlot {
    let wrap_cipher = ChaCha20Poly1305::new(Key::from_slice(kek));
    let zero_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
    let mut cek_buf = *cek;
    let wrap_tag = wrap_cipher
        .encrypt_in_place_detached(zero_nonce, b"", &mut cek_buf)
        .expect("zero-nonce ChaCha20 wrap must succeed");

    let mut wrapped = [0u8; WRAP_LEN];
    wrapped[..NONCE_LEN].copy_from_slice(&[0u8; NONCE_LEN]);
    wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&cek_buf);
    wrapped[NONCE_LEN + CEK_LEN..].copy_from_slice(wrap_tag.as_slice());

    WrappedCekSlot {
        key_id,
        wrap_scheme: WRAP_SCHEME_SYMMETRIC_CHACHA20POLY1305,
        wrapped,
    }
}

/// Build the encrypted `.lmod` container from parsed components.
fn build_encrypted_container(
    original_bytes: &[u8],
    container: &Container,
    payload: &[u8],
    eh: &EncHeader,
    cek_bytes: &[u8; CEK_LEN],
    eh_len: usize,
) -> Result<Vec<u8>, EncError> {
    let hdr = container.header();
    let eh_len_u32 = eh_len as u32;
    let cek = Key::from_slice(cek_bytes);

    let layout = compute_layout(
        hdr.abi_hash, hdr.modinfo_len, hdr.code_len, hdr.rodata_len,
        hdr.data_len, hdr.bss_len, hdr.reloc_count, eh_len_u32,
    );

    let total = layout.total_len as usize;
    let mut out = vec![0u8; total];

    let mut header_out = layout;
    header_out.format_ver = FORMAT_VER;
    header_out.flags = hdr.flags | LMOD_FLAG_ENCRYPTED;
    encode_header(&mut out, &header_out);

    let mut eh_bytes = vec![0u8; eh_len];
    encode_enc_header(&mut eh_bytes, eh).expect("enc-header encoding must succeed");
    out[HEADER_SIZE as usize..HEADER_SIZE as usize + eh_len].copy_from_slice(&eh_bytes);

    let mi = container.modinfo();
    out[layout.modinfo_off as usize..layout.modinfo_off as usize + mi.len()].copy_from_slice(mi);

    let orig_hdr = container.header();
    let ro = orig_hdr.reloc_off as usize;
    let reloc_count = orig_hdr.reloc_count as usize;
    let reloc_bytes = reloc_count * lmod::reloc::RELOC_ENTRY_SIZE as usize;
    if reloc_bytes > 0 && ro + reloc_bytes <= original_bytes.len() {
        out[layout.reloc_off as usize..layout.reloc_off as usize + reloc_bytes]
            .copy_from_slice(&original_bytes[ro..ro + reloc_bytes]);

        let old_code_off = orig_hdr.code_off;
        let new_code_off = layout.code_off;
        if new_code_off != old_code_off {
            let delta = new_code_off.wrapping_sub(old_code_off);
            let reloc_start = layout.reloc_off as usize;
            for i in 0..reloc_count {
                let entry_off = reloc_start + i * lmod::reloc::RELOC_ENTRY_SIZE as usize;
                let site_off = u32::from_le_bytes(
                    out[entry_off..entry_off + 4].try_into().unwrap()
                );
                let adjusted = site_off.wrapping_add(delta);
                out[entry_off..entry_off + 4].copy_from_slice(&adjusted.to_le_bytes());
            }
        }
    }

    // Build AAD.
    let mut aad = Vec::new();
    aad.extend_from_slice(&out[..HEADER_SIZE as usize]);
    let tag_off_in_eh = 4 + NONCE_LEN;
    eh_bytes[tag_off_in_eh..tag_off_in_eh + TAG_LEN].fill(0);
    aad.extend_from_slice(&eh_bytes);
    aad.extend_from_slice(mi);
    if reloc_bytes > 0 {
        aad.extend_from_slice(&out[layout.reloc_off as usize..layout.reloc_off as usize + reloc_bytes]);
    }

    // Encrypt payload.
    let cipher = ChaCha20Poly1305::new(cek);
    let aead_nonce = Nonce::from_slice(&eh.nonce);
    let ciphertext = cipher.encrypt(aead_nonce, Payload { msg: payload, aad: &aad })
        .map_err(|_| EncError::PayloadEncryptionFailed)?;

    // Write encrypted code.
    let co = layout.code_off as usize;
    out[co..co + hdr.code_len as usize].copy_from_slice(&ciphertext[..hdr.code_len as usize]);

    if hdr.rodata_len > 0 {
        let ro_start = co + hdr.code_len as usize;
        let ro_len = hdr.rodata_len as usize;
        out[ro_start..ro_start + ro_len].copy_from_slice(
            &ciphertext[hdr.code_len as usize..hdr.code_len as usize + ro_len]
        );
    }

    if hdr.data_len > 0 {
        let data_start = layout.data_off as usize;
        let payload_off = (hdr.code_len + hdr.rodata_len) as usize;
        out[data_start..data_start + hdr.data_len as usize].copy_from_slice(
            &ciphertext[payload_off..payload_off + hdr.data_len as usize]
        );
    }

    // Write AEAD tag into enc-header.
    let aead_tag = &ciphertext[ciphertext.len() - TAG_LEN..];
    let tag_off_in_output = HEADER_SIZE as usize + tag_off_in_eh;
    out[tag_off_in_output..tag_off_in_output + TAG_LEN].copy_from_slice(aead_tag);

    Ok(out)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Encrypt a `.lmod` container in fleet mode.
///
/// `input` is the raw bytes of a packed `.lmod`.  Returns the encrypted
/// container bytes on success.
pub fn encrypt_fleet(input: &[u8], kek: &[u8; 32]) -> Result<Vec<u8>, EncError> {
    let container = Container::parse(input).map_err(|_| EncError::InvalidContainer)?;

    let mut cek_bytes = [0u8; CEK_LEN];
    OsRng.fill_bytes(&mut cek_bytes);
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);

    let payload = {
        let mut p = Vec::with_capacity(
            container.code().len() + container.rodata().len() + container.data().len()
        );
        p.extend_from_slice(container.code());
        p.extend_from_slice(container.rodata());
        p.extend_from_slice(container.data());
        p
    };

    let slots = vec![wrap_cek(kek, &cek_bytes, 0)];

    let eh = EncHeader {
        enc_mode: EncMode::Fleet,
        aead_id: AEAD_CHACHA20POLY1305,
        nonce,
        tag: [0u8; TAG_LEN],
        wrapped_slots: slots,
    };
    let eh_len = enc_header_len(eh.wrapped_slots.len());

    build_encrypted_container(input, &container, &payload, &eh, &cek_bytes, eh_len)
}

/// Encrypt a `.lmod` container in device mode.
///
/// `input` is the raw bytes of a packed `.lmod`.  `device_keys` is a list
/// of `(device_id, 32-byte-KEK)` pairs.  Returns the encrypted container
/// bytes on success.
pub fn encrypt_device(input: &[u8], device_keys: &[(String, [u8; 32])]) -> Result<Vec<u8>, EncError> {
    let container = Container::parse(input).map_err(|_| EncError::InvalidContainer)?;

    let mut cek_bytes = [0u8; CEK_LEN];
    OsRng.fill_bytes(&mut cek_bytes);
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);

    let payload = {
        let mut p = Vec::with_capacity(
            container.code().len() + container.rodata().len() + container.data().len()
        );
        p.extend_from_slice(container.code());
        p.extend_from_slice(container.rodata());
        p.extend_from_slice(container.data());
        p
    };

    let slots: Vec<WrappedCekSlot> = device_keys.iter().enumerate()
        .map(|(idx, (_, kek))| wrap_cek(kek, &cek_bytes, idx as u64))
        .collect();

    let eh = EncHeader {
        enc_mode: EncMode::Device,
        aead_id: AEAD_CHACHA20POLY1305,
        nonce,
        tag: [0u8; TAG_LEN],
        wrapped_slots: slots,
    };
    let eh_len = enc_header_len(eh.wrapped_slots.len());

    build_encrypted_container(input, &container, &payload, &eh, &cek_bytes, eh_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lmod::header::HEADER_SIZE;

    /// Build a minimal plaintext .lmod whose code section carries `code_bytes`.
    fn make_plaintext_lmod_with_code(code_bytes: &[u8]) -> Vec<u8> {
        let code_len = code_bytes.len() as u32;
        let layout = compute_layout(42, 16, code_len, 0, 0, 0, 0, 0);
        let total = layout.total_len as usize;
        let mut buf = vec![0u8; total];
        encode_header(&mut buf, &layout);
        let mi_start = layout.modinfo_off as usize;
        buf[mi_start..mi_start + 4].copy_from_slice(b"MODI");
        let co = layout.code_off as usize;
        buf[co..co + code_bytes.len()].copy_from_slice(code_bytes);
        buf
    }

    /// Return the code-slice bytes from a pre-encryption container.
    fn input_code(input: &[u8]) -> Vec<u8> {
        let c = Container::parse(input).unwrap();
        c.code().to_vec()
    }

    /// Return the code+rodata+data payload as the library saw it.
    fn original_payload(input: &[u8]) -> Vec<u8> {
        let c = Container::parse(input).unwrap();
        let mut p = Vec::with_capacity(
            c.code().len() + c.rodata().len() + c.data().len()
        );
        p.extend_from_slice(c.code());
        p.extend_from_slice(c.rodata());
        p.extend_from_slice(c.data());
        p
    }

    /// Inverse of `wrap_cek`: unwrap the CEK using the KEK.
    fn unwrap_cek_with_kek(kek: &[u8; 32], slot: &WrappedCekSlot) -> Result<[u8; CEK_LEN], ()> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(kek));
        let zero_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
        let mut cek = [0u8; CEK_LEN];
        cek.copy_from_slice(&slot.wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN]);
        let tag = &slot.wrapped[NONCE_LEN + CEK_LEN..];
        cipher
            .decrypt_in_place_detached(zero_nonce, b"", &mut cek, tag.into())
            .map_err(|_| ())?;
        Ok(cek)
    }

    /// Reconstruct the AEAD AAD as the loader would, then decrypt the payload.
    /// Returns the original plaintext (code+rodata+data) on success.
    fn decrypt_payload(
        encrypted: &[u8],
        cek: &[u8; CEK_LEN],
    ) -> Result<Vec<u8>, ()> {
        let c = Container::parse(encrypted).map_err(|_| ())?;
        let hdr = c.header();

        // Collect the ciphertext (code + rodata + data, contiguous).
        let mut ct = Vec::with_capacity(
            hdr.code_len as usize + hdr.rodata_len as usize + hdr.data_len as usize
        );
        ct.extend_from_slice(c.code());
        ct.extend_from_slice(c.rodata());
        ct.extend_from_slice(c.data());

        // Read the AEAD tag from the enc-header.
        let eh_bytes = &encrypted[HEADER_SIZE as usize..];
        let eh = lmod::enc::decode_enc_header(eh_bytes).ok_or(())?;
        let tag = eh.tag;

        // Reconstruct AAD: header + enc-header (tag zeroed) + modinfo + reloc.
        let mut aad = Vec::new();
        aad.extend_from_slice(&encrypted[..HEADER_SIZE as usize]);
        let mut eh_for_aad = eh_bytes[..lmod::enc::enc_header_len(eh.wrapped_slots.len())].to_vec();
        let tag_off_in_eh = 4 + NONCE_LEN;
        eh_for_aad[tag_off_in_eh..tag_off_in_eh + TAG_LEN].fill(0);
        aad.extend_from_slice(&eh_for_aad);
        aad.extend_from_slice(c.modinfo());
        let reloc_bytes = (hdr.reloc_count as usize)
            .checked_mul(lmod::reloc::RELOC_ENTRY_SIZE as usize)
            .unwrap_or(0);
        if reloc_bytes > 0 {
            let ro = hdr.reloc_off as usize;
            if ro + reloc_bytes <= encrypted.len() {
                aad.extend_from_slice(&encrypted[ro..ro + reloc_bytes]);
            }
        }

        // AEAD-decrypt (Chacha20Poly1305::decrypt appends tag internally).
        let ct_with_tag = {
            let mut buf = ct;
            buf.extend_from_slice(&tag);
            buf
        };
        let cipher = ChaCha20Poly1305::new(Key::from_slice(cek));
        let nonce = Nonce::from_slice(&eh.nonce);
        cipher
            .decrypt(nonce, Payload { msg: &ct_with_tag, aad: &aad })
            .map_err(|_| ())
    }

    #[test]
    fn fleet_encrypt_roundtrips_and_actually_encrypts() {
        let input = make_plaintext_lmod_with_code(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let kek = [0xab; 32];

        let enc = encrypt_fleet(&input, &kek).unwrap();
        let c = Container::parse(&enc).unwrap();

        // (a) Payload is NOT plaintext — encryption actually happened.
        assert_ne!(c.code(), &input_code(&input)[..],
            "code must be encrypted, not copied");

        // (b) Unwrap the CEK with the KEK.
        let eh = lmod::enc::decode_enc_header(&enc[HEADER_SIZE as usize..]).unwrap();
        let cek = unwrap_cek_with_kek(&kek, &eh.wrapped_slots[0])
            .expect("CEK must unwrap with correct KEK");

        // (c) Decrypt with CEK + loader-style AAD; must recover original payload.
        let pt = decrypt_payload(&enc, &cek)
            .expect("must authenticate + decrypt");
        assert_eq!(pt, original_payload(&input),
            "decrypt must recover original payload");

        // (d) Tamper a ciphertext byte → AEAD authentication fails.
        let mut bad = enc.clone();
        let co = c.header().code_off as usize;
        bad[co] ^= 0x01;
        let result = decrypt_payload(&bad, &cek);
        assert!(result.is_err(), "flipped ciphertext must fail AEAD auth");
    }

    #[test]
    fn invalid_input_rejected() {
        let result = encrypt_fleet(b"not an lmod", &[0; 32]);
        assert_eq!(result, Err(EncError::InvalidContainer));
    }

    #[test]
    fn device_all_slots_unwrap_to_same_cek() {
        // Every device slot must wrap the same CEK under its respective KEK.
        let input = make_plaintext_lmod_with_code(&[0x11, 0x22, 0x33, 0x44]);
        let keys = vec![
            ("dev-a".into(), [0xAAu8; 32]),
            ("dev-b".into(), [0xBBu8; 32]),
            ("dev-c".into(), [0xCCu8; 32]),
        ];
        let enc = encrypt_device(&input, &keys).unwrap();

        let eh = lmod::enc::decode_enc_header(&enc[HEADER_SIZE as usize..]).unwrap();
        assert_eq!(eh.wrapped_slots.len(), 3);

        let cek0 = unwrap_cek_with_kek(&keys[0].1, &eh.wrapped_slots[0])
            .expect("slot 0 must unwrap");
        let cek1 = unwrap_cek_with_kek(&keys[1].1, &eh.wrapped_slots[1])
            .expect("slot 1 must unwrap");
        let cek2 = unwrap_cek_with_kek(&keys[2].1, &eh.wrapped_slots[2])
            .expect("slot 2 must unrap");

        assert_eq!(cek0, cek1, "all device slots must wrap the same CEK");
        assert_eq!(cek1, cek2, "all device slots must wrap the same CEK");

        // Verify round-trip with each unwrapped CEK.
        for cek in &[cek0, cek1, cek2] {
            let pt = decrypt_payload(&enc, cek)
                .expect("each unwrapped CEK must decrypt");
            assert_eq!(pt, original_payload(&input),
                "decrypt with each slot's CEK must recover original payload");
        }
    }

    #[test]
    fn wrong_key_fails_to_unwrap_cek() {
        let input = make_plaintext_lmod_with_code(&[0x11, 0x22, 0x33, 0x44]);
        let keys = vec![("dev-a".into(), [0xAAu8; 32])];
        let enc = encrypt_device(&input, &keys).unwrap();

        let eh = lmod::enc::decode_enc_header(&enc[HEADER_SIZE as usize..]).unwrap();
        let wrong_kek = [0xFFu8; 32];
        let result = unwrap_cek_with_kek(&wrong_kek, &eh.wrapped_slots[0]);
        assert!(result.is_err(), "wrong key must fail to unwrap CEK");
    }

    #[test]
    fn different_nonces_produce_different_output() {
        let input = make_plaintext_lmod_with_code(&[0x11, 0x22, 0x33, 0x44]);
        let kek = [0xabu8; 32];
        let a = encrypt_fleet(&input, &kek).unwrap();
        let b = encrypt_fleet(&input, &kek).unwrap();
        assert_ne!(a, b, "two encryptions of same input must differ");
    }
}
