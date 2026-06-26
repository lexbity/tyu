/// FNV-1a 64-bit hash.
///
/// Standard FNV-1a parameters: offset basis = 14695981039346656037,
/// prime = 1099511628211.
pub fn fnv1a_u64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// Return the loader symbol hash for a linked symbol name.
///
/// Runtime and word labels are emitted as `w_<16 hex digits>`, where the suffix
/// is already the ABI symbol hash. Other ABI symbols use FNV-1a over the symbol
/// name itself.
pub fn linked_symbol_hash(name: &[u8]) -> u64 {
    if let Some(hash) = parse_mangled_word_hash(name) {
        hash
    } else {
        fnv1a_u64(name)
    }
}

/// Parse `w_<16 lowercase/uppercase hex digits>` into the embedded hash.
pub fn parse_mangled_word_hash(name: &[u8]) -> Option<u64> {
    let hex = name.strip_prefix(b"w_")?;
    if hex.len() != 16 || !hex.iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = 0u64;
    for &b in hex {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        out = (out << 4) | u64::from(digit);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_u64_empty() {
        assert_eq!(fnv1a_u64(b""), 14695981039346656037);
    }

    #[test]
    fn fnv1a_u64_hello() {
        // Verified against Python reference implementation.
        assert_eq!(fnv1a_u64(b"hello"), 0xa430d84680aabd0b);
    }

    #[test]
    fn fnv1a_u64_deterministic() {
        let a = fnv1a_u64(b"main");
        let b = fnv1a_u64(b"main");
        assert_eq!(a, b);
    }

    #[test]
    fn fnv1a_u64_distinct() {
        assert_ne!(fnv1a_u64(b"main"), fnv1a_u64(b"helper"));
    }

    #[test]
    fn fnv1a_u64_u64_range() {
        // Verify it works with the full 64-bit range
        let h = fnv1a_u64(b"\x00\x01\x02\x03\x04\x05\x06\x07");
        assert_ne!(h, 0);
    }

    #[test]
    fn parse_mangled_word_hash_accepts_runtime_labels() {
        assert_eq!(
            parse_mangled_word_hash(b"w_accb676a903a06d9"),
            Some(0xaccb676a903a06d9)
        );
        assert_eq!(
            parse_mangled_word_hash(b"w_ACCB676A903A06D9"),
            Some(0xaccb676a903a06d9)
        );
    }

    #[test]
    fn linked_symbol_hash_uses_word_suffix() {
        assert_eq!(
            linked_symbol_hash(b"w_accb676a903a06d9"),
            0xaccb676a903a06d9
        );
        assert_ne!(
            linked_symbol_hash(b"w_accb676a903a06d9"),
            fnv1a_u64(b"w_accb676a903a06d9")
        );
        assert_eq!(
            linked_symbol_hash(b"__lang_ds_high"),
            fnv1a_u64(b"__lang_ds_high")
        );
    }
}
