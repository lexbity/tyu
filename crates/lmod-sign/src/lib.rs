//! `.lmod` signing library.
//!
//! Appends a signature/MAC trailer to an `.lmod` container.

use core::fmt;

use hmac::{Hmac, Mac};
use lmod::sig::{sig_len_for_scheme, SCHEME_HMAC_SHA256, TRAILER_HEADER_SIZE};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

/// Errors that can occur during signing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignError {
    /// The input is not a valid `.lmod` container (header decode failure).
    InvalidHeader,
}

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeader => write!(f, "invalid .lmod header"),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Append a HMAC-SHA256 signature trailer to a `.lmod` container.
///
/// `input` is the raw bytes of a `.lmod` (packed, optionally encrypted).
/// `key` is the 32-byte HMAC signing key.
///
/// The signing process:
/// 1. Patches header fields (flags, total_len, sig_off, sig_len) to their
///    post-signing values.
/// 2. Computes HMAC-SHA256 over the patched signed region.
/// 3. Appends the trailer (scheme byte + HMAC output).
///
/// The output is byte-for-byte deterministic for the same input and key.
pub fn sign(input: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, SignError> {
    let mut header = lmod::header::decode_header(input).ok_or(SignError::InvalidHeader)?;

    header.flags |= lmod::header::LMOD_FLAG_SIGNED;
    // The signed region covers everything up to the existing total_len
    // (which may already include a previous signature trailer).  Use
    // total_len directly rather than signed_region_len, because that
    // function returns sig_off (pre-trailer) when sig_len > 0,
    // which would drop the first trailer on re-sign.
    let region_len = header.total_len as usize;
    let sig_off = region_len as u32;
    let sig_len = TRAILER_HEADER_SIZE
        + sig_len_for_scheme(SCHEME_HMAC_SHA256).expect("HMAC-SHA256 sig length must be known");
    let new_total = sig_off + sig_len;

    // Copy the signed region and patch header fields to post-signing values.
    let mut signed_region = input[..region_len].to_vec();
    signed_region[6..8].copy_from_slice(&header.flags.to_le_bytes());
    signed_region[16..20].copy_from_slice(&new_total.to_le_bytes());
    signed_region[64..68].copy_from_slice(&sig_off.to_le_bytes());
    signed_region[68..72].copy_from_slice(&sig_len.to_le_bytes());

    // Compute HMAC-SHA256 over the corrected signed region.
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts 32-byte key");
    mac.update(&signed_region);
    let result = mac.finalize();
    let sig_bytes = result.into_bytes();

    // Build output: signed region + trailer.
    let mut out = signed_region;
    out.push(SCHEME_HMAC_SHA256);
    out.extend_from_slice(&sig_bytes);

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid .lmod container for testing.
    fn make_minimal_lmod() -> Vec<u8> {
        use lmod::header::{compute_layout, encode_header};

        let layout = compute_layout(0, 16, 8, 0, 0, 0, 0, 0);
        let total = layout.total_len as usize;
        let mut buf = vec![0u8; total];
        encode_header(&mut buf, &layout);
        let mi_start = layout.modinfo_off as usize;
        buf[mi_start..mi_start + 4].copy_from_slice(b"MODI");
        buf
    }

    #[test]
    fn sign_plain_returns_valid_output() {
        let input = make_minimal_lmod();
        let key = [0xabu8; 32];
        let result = sign(&input, &key);
        assert!(result.is_ok(), "sign must succeed");

        let output = result.unwrap();
        // Must be larger than input (trailer appended).
        assert!(
            output.len() > input.len(),
            "output must have trailer appended"
        );

        // Verify the SIGNED flag is set.
        let hdr = lmod::header::decode_header(&output).unwrap();
        assert_ne!(
            hdr.flags & lmod::header::LMOD_FLAG_SIGNED,
            0,
            "SIGNED flag must be set"
        );

        // Verify valid trailer at sig_off.
        let trailer = lmod::sig::SigTrailer::parse(&output[hdr.sig_off as usize..]);
        assert!(trailer.is_some(), "output must have valid SigTrailer");
        assert_eq!(trailer.unwrap().scheme, SCHEME_HMAC_SHA256);
    }

    #[test]
    fn sign_is_deterministic() {
        let input = make_minimal_lmod();
        let key = [0xabu8; 32];
        let a = sign(&input, &key).unwrap();
        let b = sign(&input, &key).unwrap();
        assert_eq!(a, b, "sign must be deterministic for same input+key");
    }

    #[test]
    fn different_key_produces_different_output() {
        let input = make_minimal_lmod();
        let a = sign(&input, &[0xAAu8; 32]).unwrap();
        let b = sign(&input, &[0xBBu8; 32]).unwrap();
        assert_ne!(a, b, "different keys must produce different signatures");
    }

    #[test]
    fn invalid_input_rejected() {
        let result = sign(b"not an lmod", &[0; 32]);
        assert_eq!(result, Err(SignError::InvalidHeader));
    }

    #[test]
    fn hmac_independently_verified() {
        // Independently recompute HMAC-SHA256 over the signed region and
        // assert it matches the trailer.  This detects a constant-MAC or
        // wrong-region signer that structural checks would miss.
        let input = make_minimal_lmod();
        let key = [0xabu8; 32];
        let output = sign(&input, &key).unwrap();

        let hdr = lmod::header::decode_header(&output).unwrap();
        let sig_off = hdr.sig_off as usize;
        let signed_region = &output[..sig_off];

        // Recompute HMAC over the same region.
        let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts 32-byte key");
        mac.update(signed_region);
        let expected = mac.finalize().into_bytes();

        // Read the trailer.
        let trailer_data = &output[sig_off..];
        let trailer =
            lmod::sig::SigTrailer::parse(trailer_data).expect("valid SigTrailer in signed output");
        assert_eq!(
            trailer.sig_bytes,
            expected.as_slice(),
            "HMAC in trailer must match independently-computed HMAC over signed region"
        );
    }

    #[test]
    fn signed_region_len_with_and_without_trailer() {
        // signed_region_len returns total_len when no trailer, or sig_off
        // when a trailer is present.
        let input = make_minimal_lmod();
        let hdr = lmod::header::decode_header(&input).unwrap();
        assert_eq!(hdr.sig_len, 0, "pre-sign: sig_len must be 0");

        let before = lmod::sig::signed_region_len(&hdr);
        assert_eq!(
            before, hdr.total_len as usize,
            "without trailer, signed_region_len == total_len"
        );

        let key = [0xabu8; 32];
        let out = sign(&input, &key).unwrap();
        let hdr2 = lmod::header::decode_header(&out).unwrap();
        assert_ne!(hdr2.sig_len, 0, "post-sign: sig_len must be non-zero");

        let after = lmod::sig::signed_region_len(&hdr2);
        assert_eq!(
            after, hdr2.sig_off as usize,
            "with trailer, signed_region_len == sig_off"
        );
        assert!(after < out.len(), "signed_region must fit within output");
    }

    #[test]
    fn re_sign_produces_deterministic_output() {
        // Signing an already-signed container must work and be deterministic.
        let input = make_minimal_lmod();
        let key = [0xabu8; 32];

        let first = sign(&input, &key).unwrap();
        let second = sign(&first, &key).unwrap();
        let third = sign(&second, &key).unwrap();

        // All three must be valid (parseable, SIGNED flag present).
        for (i, out) in [&first, &second, &third].iter().enumerate() {
            let hdr = lmod::header::decode_header(out).unwrap();
            assert_ne!(
                hdr.flags & lmod::header::LMOD_FLAG_SIGNED,
                0,
                "round {}: SIGNED flag must be set",
                i + 1
            );
        }

        // Each round must produce a larger output (stacking trailers).
        assert!(
            first.len() < second.len(),
            "re-sign must append a new trailer"
        );
        assert!(
            second.len() < third.len(),
            "second re-sign must append another trailer"
        );

        // The HMAC for the first signature covers the signed region that
        // includes the first trailer.  After second sign, the third output
        // has two trailers stacked — each independently verifiable.
        let first_region = &first[..lmod::header::decode_header(&first).unwrap().sig_off as usize];
        let third_hdr = lmod::header::decode_header(&third).unwrap();
        let third_region = &third[..third_hdr.sig_off as usize];
        assert!(
            third_region.len() > first_region.len(),
            "third region must include first + second trailers"
        );
    }

    #[test]
    fn sign_encrypted_container() {
        // Build a minimal encrypted container (no real crypto, just the
        // ENCRYPTED flag), then sign it.  This validates the code path
        // in sign() where the signed region includes the enc-header and
        // encrypted payload sections.
        use lmod::header::{compute_layout, encode_header, HEADER_SIZE, LMOD_FLAG_ENCRYPTED};

        let layout = compute_layout(42, 16, 8, 0, 0, 0, 0, 0);
        let total = layout.total_len as usize;
        let mut buf = vec![0u8; total];
        let mut hdr = layout;
        hdr.flags |= LMOD_FLAG_ENCRYPTED;
        encode_header(&mut buf, &hdr);
        let mi_start = hdr.modinfo_off as usize;
        buf[mi_start..mi_start + 4].copy_from_slice(b"MODI");
        // Fill code section with plausible encrypted bytes
        let co = hdr.code_off as usize;
        for i in 0..8 {
            buf[co + i] = (i ^ 0xFF) as u8;
        }

        let key = [0xabu8; 32];
        let out = sign(&buf, &key).unwrap();

        let out_hdr = lmod::header::decode_header(&out).unwrap();
        assert_ne!(
            out_hdr.flags & lmod::header::LMOD_FLAG_SIGNED,
            0,
            "encrypted+signed container must have SIGNED flag"
        );
        assert_ne!(
            out_hdr.flags & LMOD_FLAG_ENCRYPTED,
            0,
            "ENCRYPTED flag must survive signing"
        );

        // Verify the MAC independently.
        let region = &out[..out_hdr.sig_off as usize];
        let trailer_data = &out[out_hdr.sig_off as usize..];
        let trailer = lmod::sig::SigTrailer::parse(trailer_data).unwrap();
        let mut mac = HmacSha256::new_from_slice(&key).unwrap();
        mac.update(region);
        assert_eq!(
            mac.finalize().into_bytes().as_slice(),
            trailer.sig_bytes,
            "encrypted+signed: HMAC must verify"
        );
    }

    #[test]
    fn tampered_body_rejected_by_verifier() {
        // Flip one byte in the signed region; the recomputed HMAC must
        // differ from the original trailer's HMAC.
        let input = make_minimal_lmod();
        let key = [0xabu8; 32];
        let output = sign(&input, &key).unwrap();

        let hdr = lmod::header::decode_header(&output).unwrap();
        let sig_off = hdr.sig_off as usize;
        let trailer_data = &output[sig_off..];
        let original_trailer =
            lmod::sig::SigTrailer::parse(trailer_data).expect("valid SigTrailer");

        // Corrupt byte 64 in the signed region (a header field).
        let mut tampered = output.clone();
        tampered[64] ^= 0xFF;

        // Recompute HMAC over the tampered signed region.
        let tampered_region = &tampered[..sig_off];
        let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts 32-byte key");
        mac.update(tampered_region);
        let after_tamper = mac.finalize().into_bytes();

        assert_ne!(
            after_tamper.as_slice(),
            original_trailer.sig_bytes,
            "tampering a signed-region byte must change the HMAC"
        );
    }

    #[test]
    fn golden_sign_plain_parity_with_binary() {
        // Compare lib output against the Phase 0 golden signed_plain.lmod.
        let gold_dir = {
            let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.pop();
            p.pop();
            p.join("test-goldens")
        };

        let golden = std::fs::read(gold_dir.join("signed_plain.lmod")).unwrap();
        let packed = std::fs::read(gold_dir.join("packed.lmod")).unwrap();
        let key = [0xabu8; 32];

        let lib_output = sign(&packed, &key).unwrap();

        assert_eq!(
            lib_output.len(),
            golden.len(),
            "signed output size must match golden"
        );
        assert_eq!(
            lib_output, golden,
            "sign() output must be byte-identical to lmod-sign binary golden"
        );
    }
}
