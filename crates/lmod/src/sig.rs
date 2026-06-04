//! Signature trailer codec for the `.lmod` container.
//!
//! Canonical definition: module-format-and-loading.md §3 (trailer), §6 (integrity/trust).
//!
//! The signed region is `[Header ‖ modinfo ‖ code ‖ rodata ‖ data ‖ reloc]`
//! — everything except the trailer.  The trailer carries a scheme tag and the
//! raw signature/MAC bytes.

// ---------------------------------------------------------------------------
// Scheme tags
// ---------------------------------------------------------------------------

/// No signature (Tier 0).
pub const SCHEME_NONE: u8 = 0;

/// HMAC-SHA256, 32-byte MAC (Tier 1, symmetric).
pub const SCHEME_HMAC_SHA256: u8 = 1;

/// Ed25519 (reserved for future use).
pub const _SCHEME_ED25519: u8 = 2;

/// Size of an HMAC-SHA256 signature in bytes.
pub const HMAC_SHA256_LEN: u32 = 32;

/// Minimum trailer size: 1 byte scheme + 0 bytes sig.
pub const TRAILER_HEADER_SIZE: u32 = 1;

// ---------------------------------------------------------------------------
// SigTrailer — decoded view
// ---------------------------------------------------------------------------

/// A decoded signature trailer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigTrailer<'a> {
    pub scheme: u8,
    pub sig_bytes: &'a [u8],
}

impl<'a> SigTrailer<'a> {
    /// Parse a trailer from raw bytes.
    /// Returns `None` if truncated or scheme is unrecognised.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        let scheme = data[0];
        let sig_len = sig_len_for_scheme(scheme)?;
        if (data.len() as u32) < TRAILER_HEADER_SIZE + sig_len {
            return None;
        }
        Some(SigTrailer {
            scheme,
            sig_bytes: &data[1..1 + sig_len as usize],
        })
    }

    /// The total byte size of this trailer on the wire.
    pub fn wire_size(&self) -> u32 {
        TRAILER_HEADER_SIZE + sig_len_for_scheme(self.scheme).unwrap_or(0)
    }
}

pub fn sig_len_for_scheme(scheme: u8) -> Option<u32> {
    match scheme {
        SCHEME_NONE => Some(0),
        SCHEME_HMAC_SHA256 => Some(HMAC_SHA256_LEN),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Signed region computation
// ---------------------------------------------------------------------------

/// Return the byte range `[start, end)` of the signed region within a
/// `.lmod` container, given the container's own header values.
///
/// The signed region covers everything except the trailer:
/// `[0, sig_off)`  (or `[0, total_len)` if no trailer is present).
pub fn signed_region_len(header: &crate::header::LmodHeader) -> usize {
    if header.sig_len > 0 {
        header.sig_off as usize
    } else {
        header.total_len as usize
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn scheme_none_length() {
        assert_eq!(sig_len_for_scheme(SCHEME_NONE), Some(0));
    }

    #[test]
    fn scheme_hmac_sha256_length() {
        assert_eq!(sig_len_for_scheme(SCHEME_HMAC_SHA256), Some(32));
    }

    #[test]
    fn unknown_scheme_returns_none() {
        assert_eq!(sig_len_for_scheme(99), None);
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(SigTrailer::parse(b"").is_none());
    }

    #[test]
    fn parse_hmac_trailer() {
        let mut data = vec![SCHEME_HMAC_SHA256];
        data.extend_from_slice(&[0xab; 32]); // 32-byte signature
        let t = SigTrailer::parse(&data).unwrap();
        assert_eq!(t.scheme, SCHEME_HMAC_SHA256);
        assert_eq!(t.sig_bytes.len(), 32);
    }

    #[test]
    fn parse_truncated_hmac_returns_none() {
        // Only 1 byte of the 32-byte signature present.
        let data = [SCHEME_HMAC_SHA256, 0xab];
        assert!(SigTrailer::parse(&data).is_none());
    }

    #[test]
    fn parse_scheme_none_trailer() {
        let data = [SCHEME_NONE];
        let t = SigTrailer::parse(&data).unwrap();
        assert_eq!(t.scheme, SCHEME_NONE);
        assert!(t.sig_bytes.is_empty());
    }

    #[test]
    fn wire_size_hmac() {
        let t = SigTrailer {
            scheme: SCHEME_HMAC_SHA256,
            sig_bytes: &[0; 32],
        };
        assert_eq!(t.wire_size(), 33); // 1 + 32
    }

    #[test]
    fn wire_size_none() {
        let t = SigTrailer {
            scheme: SCHEME_NONE,
            sig_bytes: &[],
        };
        assert_eq!(t.wire_size(), 1);
    }
}
