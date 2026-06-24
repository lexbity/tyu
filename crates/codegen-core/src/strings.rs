//! Shared string-literal infrastructure.
//!
//! Provides the escape-sequence decoder and the maximum-string-capacity
//! constant used by all three codegen backends.  Single source of truth
//! for string lowering (NFR-MAINT-3): no backend carries its own decoder.

use frontend::{fixed::FixedVec, span::Span};

/// Maximum distinct interned string literals per compilation unit.
///
/// Every backend backs this with a fixed-size `[Span; STR_TABLE_CAP]` array.
/// This constant is the single source for that capacity (DEBT-3).
pub const STR_TABLE_CAP: usize = 128;

/// Decode a double-quoted source string span into its literal byte content,
/// resolving standard escape sequences (`\n`, `\r`, `\t`, `\0`, `\\`, `\"`).
///
/// Unknown escape sequences (e.g. `\x`) pass the second character through
/// as-is (`\x` → `x`).  This is a deliberate design choice: it keeps the
/// decoder simple and avoids hard-failing on source that uses future escape
/// codes.  **If a strict mode is ever needed, add a `strict` parameter.**
///
/// Returns `None` on malformed input (missing quotes, truncated escape, or
/// decoded length exceeding the 256-byte inline limit).
///
/// This function is ISA-independent and shared across all backends.
pub fn decode_string_bytes(src: &[u8], span: Span) -> Option<FixedVec<u8, 256>> {
    if span.end <= span.start + 1 {
        return None;
    }
    let s = &src[span.start..span.end];
    if s.first().copied()? != b'"' {
        return None;
    }
    if s.last().copied()? != b'"' {
        return None;
    }
    let mut out: FixedVec<u8, 256> = FixedVec::new();
    let mut i = 1usize;
    while i + 1 < s.len() {
        let b = s[i];
        if b == b'\\' {
            i += 1;
            if i + 1 >= s.len() {
                return None;
            }
            let e = s[i];
            let v = match e {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'0' => 0,
                b'\\' => b'\\',
                b'"' => b'"',
                _ => e,
            };
            out.push(v).ok()?;
            i += 1;
            continue;
        }
        out.push(b).ok()?;
        i += 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` is the raw source bytes including double quotes.
    /// Returns a Span spanning the entire text.
    fn span_all(text: &[u8]) -> Span {
        Span::new(0, text.len())
    }

    fn decode(text: &[u8]) -> Option<FixedVec<u8, 256>> {
        decode_string_bytes(text, span_all(text))
    }

    fn fv_eq(fv: &FixedVec<u8, 256>, expected: &[u8]) -> bool {
        fv.len() == expected.len() && fv.iter().zip(expected.iter()).all(|(a, b)| a == b)
    }

    #[test]
    fn empty_string() {
        let bytes = decode(b"\"\"").unwrap();
        assert_eq!(bytes.len(), 0);
    }

    #[test]
    fn plain_text() {
        let bytes = decode(b"\"hello\"").unwrap();
        assert!(fv_eq(&bytes, b"hello"));
    }

    #[test]
    fn escape_newline() {
        let bytes = decode(b"\"a\\nb\"").unwrap();
        assert!(fv_eq(&bytes, b"a\nb"));
    }

    #[test]
    fn escape_tab() {
        let bytes = decode(b"\"a\\tb\"").unwrap();
        assert!(fv_eq(&bytes, b"a\tb"));
    }

    #[test]
    fn escape_backslash() {
        let bytes = decode(b"\"a\\\\b\"").unwrap();
        assert!(fv_eq(&bytes, b"a\\b"));
    }

    #[test]
    fn escape_quote() {
        let bytes = decode(b"\"a\\\"b\"").unwrap();
        assert!(fv_eq(&bytes, b"a\"b"));
    }

    #[test]
    fn escape_null() {
        let bytes = decode(b"\"a\\0b\"").unwrap();
        assert!(fv_eq(&bytes, b"a\0b"));
    }

    #[test]
    fn escape_carriage_return() {
        let bytes = decode(b"\"a\\rb\"").unwrap();
        assert!(fv_eq(&bytes, b"a\rb"));
    }

    #[test]
    fn unknown_escape_passes_through() {
        let bytes = decode(b"\"\\x\"").unwrap();
        assert!(fv_eq(&bytes, b"x"));
    }

    #[test]
    fn missing_leading_quote() {
        assert!(decode_string_bytes(b"hello\"", span_all(b"hello\"")).is_none());
    }

    #[test]
    fn missing_trailing_quote() {
        assert!(decode_string_bytes(b"\"hello", span_all(b"\"hello")).is_none());
    }

    #[test]
    fn truncated_escape() {
        assert!(decode_string_bytes(b"\"\\", span_all(b"\"\\")).is_none());
    }

    // -----------------------------------------------------------------------
    // Boundary tests: 256-byte capacity, nonzero span offset, trailing \ case
    // -----------------------------------------------------------------------

    /// Helper: build a quoted string of `n` ASCII 'a' bytes.
    fn string_of_len(n: usize) -> [u8; 260] {
        let mut s = [0u8; 260];
        s[0] = b'"';
        for i in 0..n.min(258) {
            s[i + 1] = b'a';
        }
        s[n + 1] = b'"';
        s
    }

    #[test]
    fn decode_256_bytes_exact_fit() {
        // FixedVec<u8, 256> has capacity 256.  256 payload 'a' + 2 quotes = 258 span.
        let buf = string_of_len(256);
        let sp = Span::new(0, 258);
        let r = decode_string_bytes(&buf, sp);
        assert!(
            r.is_some(),
            "256 payload bytes must fit exactly in FixedVec<u8, 256>"
        );
        assert_eq!(r.unwrap().len(), 256);
    }

    #[test]
    fn decode_257_bytes_exceeds_capacity() {
        // 257 payload 'a' + 2 quotes = 259; payload overflows FixedVec<u8, 256>.
        let buf = string_of_len(257);
        let sp = Span::new(0, 259);
        let r = decode_string_bytes(&buf, sp);
        assert!(
            r.is_none(),
            "257 payload bytes must exceed FixedVec<u8, 256> capacity"
        );
    }

    #[test]
    fn decode_nonzero_span_offset() {
        // String embedded at an offset within a larger buffer.
        let src = b"prefix \"hello\" suffix";
        let sp = Span::new(7, 14); // spans `"hello"`
        let r = decode_string_bytes(src, sp).unwrap();
        assert_eq!(r.len(), 5);
        assert!(fv_eq(&r, b"hello"));
    }

    #[test]
    fn decode_trailing_bare_backslash() {
        // `"a\` is 3 bytes (quote + a + backslash).
        let r = decode_string_bytes(b"\"a\\", Span::new(0, 3));
        assert!(r.is_none(), "trailing bare backslash must be rejected");
    }
}
