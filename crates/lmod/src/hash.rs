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
}
