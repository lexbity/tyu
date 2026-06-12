//! ChaCha20-Poly1305 AEAD decrypt and CEK unwrap.
//!
//! Uses the `chacha20poly1305` crate (no_std compatible).  Decrypts the
//! payload in-place and unwraps the CEK from a `WrappedCekSlot` using the
//! platform's KEK.

use lmod::enc::{NONCE_LEN, TAG_LEN, CEK_LEN, WRAP_LEN};

/// Error returned when AEAD authentication fails.
#[derive(Debug)]
pub struct AeadError;

/// Decrypt a payload in-place.
///
/// `buffer` must contain the ciphertext (without tag).  The AEAD `tag` is
/// provided separately.  On success the buffer is overwritten with plaintext.
/// On failure the buffer content is unspecified.
pub fn decrypt_payload(
    cek: &[u8; CEK_LEN],
    nonce: &[u8; NONCE_LEN],
    tag: &[u8; TAG_LEN],
    aad: &[u8],
    buffer: &mut [u8],
) -> Result<(), AeadError> {
    use chacha20poly1305::aead::{AeadInPlace, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

    let key = Key::from_slice(cek);
    let cipher = ChaCha20Poly1305::new(key);
    let aead_nonce = Nonce::from_slice(nonce);
    let aead_tag = chacha20poly1305::Tag::from_slice(tag);

    cipher
        .decrypt_in_place_detached(aead_nonce, aad, buffer, aead_tag)
        .map_err(|_| AeadError)
}

/// Unwrap a content-encryption key from a wrapped slot.
///
/// The CEK was wrapped using ChaCha20-Poly1305 under the given KEK.
/// `wrapped` is 60 bytes (nonce(12) + ciphertext(32) + tag(16)).
///
/// The nonce is read from `wrapped[..NONCE_LEN]` (the first 12 bytes of
/// the wrapped slot).  Legacy slots that were created with a deterministic
/// zero nonce will have `[0u8; 12]` in that position and will still unwrap
/// correctly — the consumer change is strictly backward-compatible.
pub fn unwrap_cek(kek: &[u8; CEK_LEN], wrapped: &[u8; WRAP_LEN]) -> Result<[u8; CEK_LEN], AeadError> {
    use chacha20poly1305::aead::{AeadInPlace, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

    let key = Key::from_slice(kek);
    let cipher = ChaCha20Poly1305::new(key);

    // wrapped layout: nonce(12) + ciphertext(32) + tag(16)
    // Read the nonce from the slot (D-14): first 12 bytes.
    let mut nonce_buf = [0u8; NONCE_LEN];
    nonce_buf.copy_from_slice(&wrapped[..NONCE_LEN]);
    let wrap_nonce = Nonce::from_slice(&nonce_buf);

    let mut cek_buf = [0u8; CEK_LEN];
    cek_buf.copy_from_slice(&wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN]);
    let tag_start = NONCE_LEN + CEK_LEN;
    let tag: &[u8; TAG_LEN] = (&wrapped[tag_start..tag_start + TAG_LEN]).try_into().unwrap();

    let aead_tag = chacha20poly1305::Tag::from_slice(tag);
    cipher
        .decrypt_in_place_detached(wrap_nonce, b"", &mut cek_buf, aead_tag)
        .map_err(|_| AeadError)?;

    Ok(cek_buf)
}

#[cfg(test)]
mod tests {
    use chacha20poly1305::KeyInit;
    use super::*;

    /// Encrypt-then-decrypt roundtrip with non-empty AAD.
    #[test]
    fn encrypt_decrypt_roundtrip_with_aad() {
        let key = [0xab; CEK_LEN];
        let nonce = [0x03; NONCE_LEN];
        let aad = b"associated data";
        let plaintext = b"hello, world!";

        use chacha20poly1305::aead::{AeadInPlace, KeyInit};
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let anonce = Nonce::from_slice(&nonce);

        let mut buf = plaintext.to_vec();
        let tag = cipher.encrypt_in_place_detached(anonce, aad, &mut buf).unwrap();

        // Now decrypt with our function.
        let mut dec_buf = buf.clone();
        let tag_arr: &[u8; TAG_LEN] = tag.as_slice().try_into().unwrap();
        decrypt_payload(&key, &nonce, tag_arr, aad, &mut dec_buf).unwrap();
        assert_eq!(&dec_buf, plaintext, "decrypted plaintext must match original");
    }

    #[test]
    fn wrong_tag_fails() {
        let key = [0xab; CEK_LEN];
        let nonce = [0x01; NONCE_LEN];
        let mut buf = [0x42u8; 16];
        let tag = [0x00; TAG_LEN]; // wrong tag

        let result = decrypt_payload(&key, &nonce, &tag, b"", &mut buf);
        assert!(result.is_err(), "wrong tag must fail decryption");
    }

    #[test]
    fn wrong_key_fails() {
        let enc_key = [0xab; CEK_LEN];
        let dec_key = [0xcd; CEK_LEN];
        let nonce = [0x02; NONCE_LEN];
        let plaintext = b"hello";

        use chacha20poly1305::aead::AeadInPlace;
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&enc_key));
        let anonce = Nonce::from_slice(&nonce);
        let mut buf = plaintext.to_vec();
        let tag = cipher.encrypt_in_place_detached(anonce, b"", &mut buf).unwrap();

        let result = decrypt_payload(&dec_key, &nonce, tag.as_slice().try_into().unwrap(), b"", &mut buf);
        assert!(result.is_err(), "wrong key must fail decryption");
    }

    #[test]
    fn unwrap_cek_roundtrip_with_nonce() {
        let kek = [0x11; CEK_LEN];
        let cek = [0x22; CEK_LEN];
        let wrap_nonce = [0x33; NONCE_LEN];

        // Wrap the CEK using a non-zero nonce.
        use chacha20poly1305::aead::AeadInPlace;
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&kek));
        let nonce = Nonce::from_slice(&wrap_nonce);
        let mut cek_buf = cek;
        let wrap_tag = cipher
            .encrypt_in_place_detached(nonce, b"", &mut cek_buf)
            .unwrap();

        // Build the wrapped slot: nonce(12) + ciphertext(32) + tag(16)
        let mut wrapped = [0u8; WRAP_LEN];
        wrapped[..NONCE_LEN].copy_from_slice(&wrap_nonce);
        wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&cek_buf);
        wrapped[NONCE_LEN + CEK_LEN..].copy_from_slice(wrap_tag.as_slice());

        // Unwrap using the new slot-nonce method.
        let unwrapped = unwrap_cek(&kek, &wrapped).unwrap();
        assert_eq!(unwrapped, cek, "unwrapped CEK must match original");
    }

    #[test]
    fn legacy_zero_nonce_slot_still_unwraps() {
        let kek = [0x11; CEK_LEN];
        let cek = [0x22; CEK_LEN];

        // Wrap with zero nonce (legacy method).
        use chacha20poly1305::aead::AeadInPlace;
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&kek));
        let zero_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
        let mut cek_buf = cek;
        let wrap_tag = cipher
            .encrypt_in_place_detached(zero_nonce, b"", &mut cek_buf)
            .unwrap();

        let mut wrapped = [0u8; WRAP_LEN];
        wrapped[..NONCE_LEN].copy_from_slice(&[0u8; NONCE_LEN]); // legacy zero nonce
        wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&cek_buf);
        wrapped[NONCE_LEN + CEK_LEN..].copy_from_slice(wrap_tag.as_slice());

        let unwrapped = unwrap_cek(&kek, &wrapped).unwrap();
        assert_eq!(unwrapped, cek, "legacy zero-nonce slot must still unwrap");
    }

    #[test]
    fn unwrap_cek_wrong_kek_fails() {
        let wrap_kek = [0xaa; CEK_LEN];
        let unwrap_kek = [0xbb; CEK_LEN];
        let cek = [0xcc; CEK_LEN];

        use chacha20poly1305::aead::AeadInPlace;
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&wrap_kek));
        let zero_nonce = Nonce::from_slice(&[0u8; NONCE_LEN]);
        let mut cek_buf = cek;
        let wrap_tag = cipher.encrypt_in_place_detached(zero_nonce, b"", &mut cek_buf).unwrap();

        let mut wrapped = [0u8; WRAP_LEN];
        wrapped[..NONCE_LEN].copy_from_slice(&[0u8; NONCE_LEN]);
        wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&cek_buf);
        wrapped[NONCE_LEN + CEK_LEN..].copy_from_slice(wrap_tag.as_slice());

        let result = unwrap_cek(&unwrap_kek, &wrapped);
        assert!(result.is_err(), "wrong KEK must fail unwrap");
    }

    // -----------------------------------------------------------------------
    // U-CR-1: Decrypt with mismatched AAD must fail (AE-3)
    // -----------------------------------------------------------------------

    #[test]
    fn decrypt_with_mismatched_aad_fails() {
        let key = [0xab; CEK_LEN];
        let nonce = [0x04; NONCE_LEN];
        let aad_a = b"correct aad";
        let aad_b = b"wrong aad";
        let plaintext = b"payload data";

        // Encrypt with aad_a.
        use chacha20poly1305::aead::AeadInPlace;
        use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let anonce = Nonce::from_slice(&nonce);
        let mut buf = plaintext.to_vec();
        let tag = cipher.encrypt_in_place_detached(anonce, aad_a, &mut buf).unwrap();
        let tag_arr: &[u8; TAG_LEN] = tag.as_slice().try_into().unwrap();

        // Decrypt with aad_b — must fail.
        let result = decrypt_payload(&key, &nonce, tag_arr, aad_b, &mut buf);
        assert!(result.is_err(), "mismatched AAD must fail decryption");
    }

    // -----------------------------------------------------------------------
    // U-CR-2: unwrap_cek with zeroed tag (truncated/invalid) must fail
    // -----------------------------------------------------------------------

    #[test]
    fn unwrap_cek_truncated_slot() {
        let kek = [0x11; CEK_LEN];
        // Build a wrapped slot with the tag region zeroed.
        let mut wrapped = [0u8; WRAP_LEN];
        // Pre-fill with a valid ciphertext portion but keep the tag zeroed.
        wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&[0x42u8; CEK_LEN]);
        // tag bytes are already zero from initialization.

        let result = unwrap_cek(&kek, &wrapped);
        assert!(result.is_err(), "zeroed tag must fail unwrap");
    }
}
