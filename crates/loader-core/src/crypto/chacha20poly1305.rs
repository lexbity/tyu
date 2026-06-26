//! Minimal ChaCha20-Poly1305 decrypt and CEK unwrap for the device loader.

use lmod::enc::{CEK_LEN, NONCE_LEN, TAG_LEN, WRAP_LEN};

#[derive(Debug)]
pub struct AeadError;

const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

pub fn decrypt_payload(
    cek: &[u8; CEK_LEN],
    nonce: &[u8; NONCE_LEN],
    tag: &[u8; TAG_LEN],
    aad: &[u8],
    buffer: &mut [u8],
) -> Result<(), AeadError> {
    let expected = poly1305_tag(cek, nonce, aad, buffer);
    if !ct_eq(&expected, tag) {
        return Err(AeadError);
    }
    chacha20_xor(cek, nonce, 1, buffer);
    Ok(())
}

pub fn unwrap_cek(
    kek: &[u8; CEK_LEN],
    wrapped: &[u8; WRAP_LEN],
) -> Result<[u8; CEK_LEN], AeadError> {
    let nonce: &[u8; NONCE_LEN] = wrapped[..NONCE_LEN].try_into().map_err(|_| AeadError)?;
    let mut cek = [0u8; CEK_LEN];
    cek.copy_from_slice(&wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN]);
    let tag_start = NONCE_LEN + CEK_LEN;
    let tag: &[u8; TAG_LEN] = wrapped[tag_start..tag_start + TAG_LEN]
        .try_into()
        .map_err(|_| AeadError)?;
    decrypt_payload(kek, nonce, tag, b"", &mut cek)?;
    Ok(cek)
}

fn chacha20_xor(key: &[u8; 32], nonce: &[u8; 12], mut counter: u32, data: &mut [u8]) {
    for chunk in data.chunks_mut(64) {
        let block = chacha20_block(key, nonce, counter);
        counter = counter.wrapping_add(1);
        for (dst, src) in chunk.iter_mut().zip(block.iter()) {
            *dst ^= *src;
        }
    }
}

fn chacha20_block(key: &[u8; 32], nonce: &[u8; 12], counter: u32) -> [u8; 64] {
    let mut state = [0u32; 16];
    state[..4].copy_from_slice(&CONSTANTS);
    for i in 0..8 {
        state[4 + i] = read_u32(&key[i * 4..i * 4 + 4]);
    }
    state[12] = counter;
    state[13] = read_u32(&nonce[0..4]);
    state[14] = read_u32(&nonce[4..8]);
    state[15] = read_u32(&nonce[8..12]);

    let mut working = state;
    for _ in 0..10 {
        quarter_round(&mut working, 0, 4, 8, 12);
        quarter_round(&mut working, 1, 5, 9, 13);
        quarter_round(&mut working, 2, 6, 10, 14);
        quarter_round(&mut working, 3, 7, 11, 15);
        quarter_round(&mut working, 0, 5, 10, 15);
        quarter_round(&mut working, 1, 6, 11, 12);
        quarter_round(&mut working, 2, 7, 8, 13);
        quarter_round(&mut working, 3, 4, 9, 14);
    }

    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&working[i].wrapping_add(state[i]).to_le_bytes());
    }
    out
}

fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(7);
}

fn poly1305_tag(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let block0 = chacha20_block(key, nonce, 0);
    let mut poly_key = [0u8; 32];
    poly_key.copy_from_slice(&block0[..32]);
    let mut poly = Poly1305::new(&poly_key);
    poly.update_aead(aad);
    poly.update_aead(ciphertext);
    let mut lengths = [0u8; 16];
    lengths[..8].copy_from_slice(&(aad.len() as u64).to_le_bytes());
    lengths[8..].copy_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    poly.compute_block(&lengths, false);
    poly.finalize()
}

#[derive(Clone, Default)]
struct Poly1305 {
    r: [u32; 5],
    h: [u32; 5],
    pad: [u32; 4],
}

impl Poly1305 {
    fn new(key: &[u8; 32]) -> Self {
        Self {
            r: [
                read_u32(&key[0..4]) & 0x3ff_ffff,
                (read_u32(&key[3..7]) >> 2) & 0x3ff_ff03,
                (read_u32(&key[6..10]) >> 4) & 0x3ff_c0ff,
                (read_u32(&key[9..13]) >> 6) & 0x3f0_3fff,
                (read_u32(&key[12..16]) >> 8) & 0x00f_ffff,
            ],
            h: [0; 5],
            pad: [
                read_u32(&key[16..20]),
                read_u32(&key[20..24]),
                read_u32(&key[24..28]),
                read_u32(&key[28..32]),
            ],
        }
    }

    fn update_aead(&mut self, data: &[u8]) {
        for chunk in data.chunks(16) {
            let mut block = [0u8; 16];
            block[..chunk.len()].copy_from_slice(chunk);
            self.compute_block(&block, false);
        }
    }

    fn compute_block(&mut self, block: &[u8; 16], partial: bool) {
        let hibit = if partial { 0 } else { 1 << 24 };
        let r0 = self.r[0];
        let r1 = self.r[1];
        let r2 = self.r[2];
        let r3 = self.r[3];
        let r4 = self.r[4];
        let s1 = r1 * 5;
        let s2 = r2 * 5;
        let s3 = r3 * 5;
        let s4 = r4 * 5;
        let mut h0 = self.h[0];
        let mut h1 = self.h[1];
        let mut h2 = self.h[2];
        let mut h3 = self.h[3];
        let mut h4 = self.h[4];

        h0 += read_u32(&block[0..4]) & 0x3ff_ffff;
        h1 += (read_u32(&block[3..7]) >> 2) & 0x3ff_ffff;
        h2 += (read_u32(&block[6..10]) >> 4) & 0x3ff_ffff;
        h3 += (read_u32(&block[9..13]) >> 6) & 0x3ff_ffff;
        h4 += (read_u32(&block[12..16]) >> 8) | hibit;

        let d0 = u64::from(h0) * u64::from(r0)
            + u64::from(h1) * u64::from(s4)
            + u64::from(h2) * u64::from(s3)
            + u64::from(h3) * u64::from(s2)
            + u64::from(h4) * u64::from(s1);
        let mut d1 = u64::from(h0) * u64::from(r1)
            + u64::from(h1) * u64::from(r0)
            + u64::from(h2) * u64::from(s4)
            + u64::from(h3) * u64::from(s3)
            + u64::from(h4) * u64::from(s2);
        let mut d2 = u64::from(h0) * u64::from(r2)
            + u64::from(h1) * u64::from(r1)
            + u64::from(h2) * u64::from(r0)
            + u64::from(h3) * u64::from(s4)
            + u64::from(h4) * u64::from(s3);
        let mut d3 = u64::from(h0) * u64::from(r3)
            + u64::from(h1) * u64::from(r2)
            + u64::from(h2) * u64::from(r1)
            + u64::from(h3) * u64::from(r0)
            + u64::from(h4) * u64::from(s4);
        let mut d4 = u64::from(h0) * u64::from(r4)
            + u64::from(h1) * u64::from(r3)
            + u64::from(h2) * u64::from(r2)
            + u64::from(h3) * u64::from(r1)
            + u64::from(h4) * u64::from(r0);

        let mut c = (d0 >> 26) as u32;
        h0 = d0 as u32 & 0x3ff_ffff;
        d1 += u64::from(c);
        c = (d1 >> 26) as u32;
        h1 = d1 as u32 & 0x3ff_ffff;
        d2 += u64::from(c);
        c = (d2 >> 26) as u32;
        h2 = d2 as u32 & 0x3ff_ffff;
        d3 += u64::from(c);
        c = (d3 >> 26) as u32;
        h3 = d3 as u32 & 0x3ff_ffff;
        d4 += u64::from(c);
        c = (d4 >> 26) as u32;
        h4 = d4 as u32 & 0x3ff_ffff;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= 0x3ff_ffff;
        h1 += c;

        self.h = [h0, h1, h2, h3, h4];
    }

    fn finalize(self) -> [u8; 16] {
        let mut h0 = self.h[0];
        let mut h1 = self.h[1];
        let mut h2 = self.h[2];
        let mut h3 = self.h[3];
        let mut h4 = self.h[4];
        let mut c = h1 >> 26;
        h1 &= 0x3ff_ffff;
        h2 += c;
        c = h2 >> 26;
        h2 &= 0x3ff_ffff;
        h3 += c;
        c = h3 >> 26;
        h3 &= 0x3ff_ffff;
        h4 += c;
        c = h4 >> 26;
        h4 &= 0x3ff_ffff;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= 0x3ff_ffff;
        h1 += c;

        let mut g0 = h0.wrapping_add(5);
        c = g0 >> 26;
        g0 &= 0x3ff_ffff;
        let mut g1 = h1.wrapping_add(c);
        c = g1 >> 26;
        g1 &= 0x3ff_ffff;
        let mut g2 = h2.wrapping_add(c);
        c = g2 >> 26;
        g2 &= 0x3ff_ffff;
        let mut g3 = h3.wrapping_add(c);
        c = g3 >> 26;
        g3 &= 0x3ff_ffff;
        let mut g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);
        let mut mask = (g4 >> 31).wrapping_sub(1);
        g0 &= mask;
        g1 &= mask;
        g2 &= mask;
        g3 &= mask;
        g4 &= mask;
        mask = !mask;
        h0 = (h0 & mask) | g0;
        h1 = (h1 & mask) | g1;
        h2 = (h2 & mask) | g2;
        h3 = (h3 & mask) | g3;
        h4 = (h4 & mask) | g4;

        h0 |= h1 << 26;
        h1 = (h1 >> 6) | (h2 << 20);
        h2 = (h2 >> 12) | (h3 << 14);
        h3 = (h3 >> 18) | (h4 << 8);

        let mut f = u64::from(h0) + u64::from(self.pad[0]);
        h0 = f as u32;
        f = u64::from(h1) + u64::from(self.pad[1]) + (f >> 32);
        h1 = f as u32;
        f = u64::from(h2) + u64::from(self.pad[2]) + (f >> 32);
        h2 = f as u32;
        f = u64::from(h3) + u64::from(self.pad[3]) + (f >> 32);
        h3 = f as u32;

        let mut tag = [0u8; 16];
        tag[0..4].copy_from_slice(&h0.to_le_bytes());
        tag[4..8].copy_from_slice(&h1.to_le_bytes());
        tag[8..12].copy_from_slice(&h2.to_le_bytes());
        tag[12..16].copy_from_slice(&h3.to_le_bytes());
        tag
    }
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use chacha20poly1305::aead::{AeadInPlace, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

    #[test]
    fn decrypt_matches_chacha20poly1305_crate() {
        let key = [0xab; CEK_LEN];
        let nonce = [0x03; NONCE_LEN];
        let aad = b"associated data";
        let mut ciphertext = b"hello, world!".to_vec();
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let tag = cipher
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), aad, &mut ciphertext)
            .unwrap();
        let tag: &[u8; TAG_LEN] = tag.as_slice().try_into().unwrap();
        decrypt_payload(&key, &nonce, tag, aad, &mut ciphertext).unwrap();
        assert_eq!(&ciphertext, b"hello, world!");
    }

    #[test]
    fn poly1305_matches_rfc_vector() {
        let key = [
            0x85, 0xd6, 0xbe, 0x78, 0x57, 0x55, 0x6d, 0x33, 0x7f, 0x44, 0x52, 0xfe, 0x42, 0xd5,
            0x06, 0xa8, 0x01, 0x03, 0x80, 0x8a, 0xfb, 0x0d, 0xb2, 0xfd, 0x4a, 0xbf, 0xf6, 0xaf,
            0x41, 0x49, 0xf5, 0x1b,
        ];
        let mut poly = Poly1305::new(&key);
        for chunk in b"Cryptographic Forum Research Group".chunks(16) {
            let mut block = [0u8; 16];
            block[..chunk.len()].copy_from_slice(chunk);
            if chunk.len() != 16 {
                block[chunk.len()] = 1;
            }
            poly.compute_block(&block, chunk.len() != 16);
        }
        assert_eq!(
            poly.finalize(),
            [
                0xa8, 0x06, 0x1d, 0xc1, 0x30, 0x51, 0x36, 0xc6, 0xc2, 0x2b, 0x8b, 0xaf, 0x0c, 0x01,
                0x27, 0xa9,
            ]
        );
    }

    #[test]
    fn unwrap_cek_roundtrip_with_nonce() {
        let kek = [0x55; CEK_LEN];
        let nonce = [0x77; NONCE_LEN];
        let cek = [0x99; CEK_LEN];
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&kek));
        let mut wrapped = [0u8; WRAP_LEN];
        wrapped[..NONCE_LEN].copy_from_slice(&nonce);
        wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN].copy_from_slice(&cek);
        let tag = cipher
            .encrypt_in_place_detached(
                Nonce::from_slice(&nonce),
                b"",
                &mut wrapped[NONCE_LEN..NONCE_LEN + CEK_LEN],
            )
            .unwrap();
        wrapped[NONCE_LEN + CEK_LEN..].copy_from_slice(tag.as_slice());
        assert_eq!(unwrap_cek(&kek, &wrapped).unwrap(), cek);
    }
}
