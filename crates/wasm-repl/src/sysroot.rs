//! The `tyu-sysroot/1` blob (PLAN-WASM-1 §Q12, DEC-16): the hosted sysroot's
//! `.def` sources packed into one versioned, content-hashed binary map. The
//! wasm boundary (P5) ships this blob and registers it as the session's
//! [`SourceTable`]; G55 verifies the shipped hashes against the repo.
//!
//! Format (little-endian, deterministic, hand-rolled per §15):
//!
//! ```text
//! "TYU-SYSROOT/1\n"          magic + version (fail-closed on anything else)
//! u32  LE                    file count
//! per file, sorted by path:
//!   u16 LE path length, path bytes (UTF-8, `/`-separated, no leading `/`)
//!   [u8; 32]                 SHA-256 of the content
//!   u32 LE content length, content bytes
//! ```
//!
//! The decoder verifies every content hash before returning the table — a
//! tampered or truncated blob is E9153, never a partial sysroot (§7.5).

use alloc::vec::Vec;
use core::ops::Range;

use crate::compile::SourceTable;
use crate::diag::code;
use crate::Diag;

/// The blob schema identifier (magic + version line).
pub const SYSROOT_SCHEMA: &[u8] = b"TYU-SYSROOT/1\n";

/// The sysroot sources the REPL session resolves imports through. Paths are
/// the canonical `.def` paths (`platform/testio.def`); sorted, so the encode
/// is byte-deterministic for a given file set.
pub struct SysrootTable {
    files: Vec<(Vec<u8>, Range<usize>)>,
    blob: Vec<u8>,
}

impl SourceTable for SysrootTable {
    fn read(&self, path: &[u8]) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, body)| &self.blob[body.clone()])
    }
}

impl SysrootTable {
    /// The number of files in the table (the blob integrity surface).
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Is the table empty? (A sysroot with no files boots nothing.)
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The (sorted) file paths — the G55 hash-verify surface.
    pub fn paths(&self) -> impl Iterator<Item = &[u8]> {
        self.files.iter().map(|(p, _)| p.as_slice())
    }
}

/// Encode a sorted file map into a `tyu-sysroot/1` blob. `files` must be
/// sorted by path and free of duplicates — `encode` enforces both (the
/// deterministic encoder is what makes the G55 byte-compare against a fresh
/// pack a valid drift check).
pub fn encode_blob(files: &[(&[u8], &[u8])]) -> Result<Vec<u8>, Diag> {
    if files.len() > u32::MAX as usize {
        return Err(err(
            frontend::span::Span::UNKNOWN,
            b"sysroot file count overflows u32",
        ));
    }
    for w in files.windows(2) {
        if w[0].0 >= w[1].0 {
            return Err(err(
                frontend::span::Span::UNKNOWN,
                b"sysroot files must be sorted by path, without duplicates",
            ));
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(SYSROOT_SCHEMA);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (path, content) in files {
        let n = path.len();
        if n > u16::MAX as usize {
            return Err(err(frontend::span::Span::UNKNOWN, b"sysroot path too long"));
        }
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(path);
        out.extend_from_slice(&sha256(content));
        out.extend_from_slice(&(content.len() as u32).to_le_bytes());
        out.extend_from_slice(content);
    }
    Ok(out)
}

/// Decode and fully verify a `tyu-sysroot/1` blob. Every content byte is
/// hash-checked; any structural or hash violation is E9153 (fail-closed).
pub fn decode_blob(blob: &[u8]) -> Result<SysrootTable, Diag> {
    let bad = |msg: &[u8]| err(frontend::span::Span::UNKNOWN, msg);
    if blob.len() < SYSROOT_SCHEMA.len() + 4 || &blob[..SYSROOT_SCHEMA.len()] != SYSROOT_SCHEMA {
        return Err(bad(
            b"sysroot blob: bad magic or version (want TYU-SYSROOT/1)",
        ));
    }
    let mut pos = SYSROOT_SCHEMA.len();
    let rd_u32 = |b: &[u8], at: usize| -> Option<u32> {
        b.get(at..at + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let count = rd_u32(blob, pos).ok_or_else(|| bad(b"sysroot blob: truncated header"))? as usize;
    pos += 4;

    let mut files: Vec<(Vec<u8>, Range<usize>)> = Vec::with_capacity(count);
    let mut prev_path: Option<&[u8]> = None;
    for i in 0..count {
        if blob.len() < pos + 2 {
            return Err(bad(b"sysroot blob: truncated entry header"));
        }
        let plen = u16::from_le_bytes([blob[pos], blob[pos + 1]]) as usize;
        pos += 2;
        if blob.len() < pos + plen + 32 + 4 {
            return Err(bad(b"sysroot blob: truncated entry"));
        }
        let path = &blob[pos..pos + plen];
        pos += plen;
        if let Some(prev) = prev_path {
            if path <= prev {
                return Err(bad(b"sysroot blob: entries not sorted by path"));
            }
        }
        prev_path = Some(path);
        let want = &blob[pos..pos + 32];
        pos += 32;
        let clen =
            rd_u32(blob, pos).ok_or_else(|| bad(b"sysroot blob: truncated length"))? as usize;
        pos += 4;
        let end = pos
            .checked_add(clen)
            .ok_or_else(|| bad(b"sysroot blob: length overflow"))?;
        if blob.len() < end {
            return Err(bad(b"sysroot blob: truncated content"));
        }
        let content = &blob[pos..end];
        if sha256(content) != want {
            return Err(bad(b"sysroot blob: content hash mismatch (E9153)"));
        }
        files.push((path.to_vec(), pos..end));
        pos = end;
        let _ = i;
    }

    Ok(SysrootTable {
        files,
        blob: blob.to_vec(),
    })
}

fn err(span: frontend::span::Span, msg: &[u8]) -> Diag {
    Diag::from_bytes(code::E9153_SYSROOT, span, msg)
}

// ---------------------------------------------------------------------------
// SHA-256 (FIPS 180-4) — the sysroot content hash. Hand-rolled to keep the
// crate dependency-free (R-A); pinned against the standard test vectors.
// ---------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// One-shot SHA-256.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());

    let mut w = [0u32; 64];
    let (chunks, _rest) = msg.as_chunks::<64>();
    for chunk in chunks {
        for (i, wi) in w.iter_mut().take(16).enumerate() {
            *wi = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    // FIPS 180-4 test vectors (the independent pin for the hand-rolled hash).
    #[test]
    fn sha256_fips_vectors() {
        let hex = |d: [u8; 32]| -> alloc::string::String {
            d.iter().map(|b| alloc::format!("{b:02x}")).collect()
        };
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // One-million 'a' (exercised via the doubling property to stay cheap
        // in the test loop is NOT the standard vector — run the real thing
        // once; 1e6 bytes hashes in well under a second).
        let million = {
            let mut v = alloc::vec![b'a'; 1_000_000];
            let _ = &mut v;
            v
        };
        assert_eq!(
            hex(sha256(&million)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn blob_round_trip_and_hash_enforcement() {
        let files: Vec<(&[u8], &[u8])> = vec![
            (b"platform/testio.def", b"def module platform/testio ...\n"),
            (b"platform/z.def", b"def module platform/z ...\n"),
        ];
        let blob = encode_blob(&files).expect("encode");
        let table = decode_blob(&blob).expect("decode");
        assert_eq!(table.len(), 2);
        assert_eq!(
            table.read(b"platform/testio.def"),
            Some(&b"def module platform/testio ...\n"[..])
        );
        assert_eq!(table.read(b"platform/missing.def"), None);
        assert_eq!(
            table.paths().collect::<Vec<_>>(),
            vec![&b"platform/testio.def"[..], &b"platform/z.def"[..]]
        );

        // Tampering: flip one content byte → the hash check must fail-closed.
        let mut tampered = blob.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(
            decode_blob(&tampered).is_err(),
            "tampered content must be rejected"
        );

        // Truncation: cut the blob short → structural rejection.
        assert!(decode_blob(&blob[..blob.len() - 1]).is_err());

        // Wrong magic/version → E9153, fail-closed.
        let mut wrong = blob.clone();
        wrong[SYSROOT_SCHEMA.len() - 2] = b'2';
        assert!(decode_blob(&wrong).is_err());

        // Unsorted entries are an encoder error (determinism guard).
        let unsorted: Vec<(&[u8], &[u8])> =
            vec![(b"platform/z.def", b"x"), (b"platform/testio.def", b"x")];
        assert!(encode_blob(&unsorted).is_err());
    }
}
