//! Thumb-2 BL / B.W branch encoding — the shared codec for the
//! `ArmThmCall` / `ArmThmJump24` reloc kinds (module-format §3.1).
//!
//! Both the packer (pre-resolving internal relocations at pack time) and the
//! on-device loader (binding import relocations at load time) patch the same
//! instruction shapes, so the bit arithmetic lives here exactly once.
//!
//! Encoding (ARMv7-M T32, 4 bytes):
//! ```text
//!   first halfword:  11110 S imm10
//!   second:          11 J1 L J2 imm11     (L = 1 for BL, 0 for B.W)
//! ```
//! with `I1 = !(J1 ^ S)`, `I2 = !(J2 ^ S)` and the byte displacement
//! `offset = S:I1:I2:imm10:imm11:'0'` in ±16 MiB, relative to `place + 4`
//! (the architecturally fixed PC bias of the branch).

/// Encode a Thumb-2 branch with byte displacement `offset` into the 4 bytes
/// at `insn`. `link` selects BL (call, L=1) or B.W (jump, L=0); when `link`
/// is `None` the site's existing L bit is preserved (a B.W never silently
/// becomes a BL — clobbering `lr` in a tail-call would corrupt the caller).
///
/// `offset` must be even and fit the signed 25-bit (±16 MiB) field; anything
/// else is `Err(())` — the caller reports it as an unsupported/out-of-range
/// relocation, never a silent wrap. The unit error is deliberate: the
/// encode/decode pair is a bit-level codec, not a diagnostic surface (the
/// callers own the relocation-context error).
#[allow(clippy::result_unit_err)]
pub fn encode_branch(insn: &mut [u8], offset: i64, link: Option<bool>) -> Result<(), ()> {
    if insn.len() < 4 {
        return Err(());
    }
    if offset & 1 != 0 {
        return Err(()); // misaligned
    }
    let half = offset >> 1; // convert bytes to halfwords
                            // Check signed 24-bit range (±16MB)
    if !(-0x800000..=0x7FFFFF).contains(&half) {
        return Err(());
    }

    let h = half as u32;
    let s: u16 = ((h >> 23) & 1) as u16;
    let i1: u16 = ((h >> 22) & 1) as u16;
    let i2: u16 = ((h >> 21) & 1) as u16;
    let j1: u16 = 1 ^ i1 ^ s;
    let j2: u16 = 1 ^ i2 ^ s;
    let imm10: u16 = ((h >> 11) & 0x3FF) as u16;
    let imm11: u16 = (h & 0x7FF) as u16;

    let existing_hw1 = u16::from_le_bytes([insn[2], insn[3]]);
    let l: u16 = match link {
        Some(true) => 0x1000,
        Some(false) => 0x0000,
        // Preserve the site's L bit (bit 12): BL stays BL, B.W stays B.W.
        None => existing_hw1 & 0x1000,
    };

    let hw0 = 0xF000u16 | (s << 10) | imm10;
    let hw1 = 0xE000u16 | l | (j1 << 13) | (j2 << 11) | imm11;

    insn[0..2].copy_from_slice(&hw0.to_le_bytes());
    insn[2..4].copy_from_slice(&hw1.to_le_bytes());
    Ok(())
}

/// Encode a Thumb-2 BL (call): `encode_branch(insn, offset, Some(true))`.
#[allow(clippy::result_unit_err)]
pub fn encode_bl(insn: &mut [u8], offset: i64) -> Result<(), ()> {
    encode_branch(insn, offset, Some(true))
}

/// Decode a Thumb-2 BL/B.W back to its byte displacement. The reverse of
/// [`encode_branch`]; the L bit is returned alongside so callers can tell a
/// call from a tail-jump. Test-side round-trip oracle — `cfg(test)` in the
/// firmware build keeps the 16 KiB device-loader text budget (G7) intact.
#[cfg(test)]
#[allow(clippy::result_unit_err)]
pub fn decode_branch(insn: &[u8]) -> Result<(i64, bool), ()> {
    if insn.len() < 4 {
        return Err(());
    }
    let hw0 = u16::from_le_bytes([insn[0], insn[1]]);
    let hw1 = u16::from_le_bytes([insn[2], insn[3]]);

    // Fixed bits: first halfword 11110, second 11xJ1xJ2 with bits 15:14 = 11.
    if (hw0 & 0xF800) != 0xF000 {
        return Err(());
    }
    if (hw1 & 0xC000) != 0xC000 {
        return Err(());
    }

    let s: u32 = ((hw0 >> 10) & 1) as u32;
    let j1: u32 = ((hw1 >> 13) & 1) as u32;
    let j2: u32 = ((hw1 >> 11) & 1) as u32;
    let imm10: u32 = (hw0 & 0x03FF) as u32;
    let imm11: u32 = (hw1 & 0x7FF) as u32;

    // I1 = J1 ^ (S ^ 1), I2 = J2 ^ (S ^ 1)
    let i1 = j1 ^ (s ^ 1);
    let i2 = j2 ^ (s ^ 1);

    let half: u32 = (s << 23) | (i1 << 22) | (i2 << 21) | (imm10 << 11) | imm11;
    // Sign-extend the 24-bit halfword field BEFORE doubling: extending after
    // the `*2` would lose the sign (the doubled 25-bit value overflows the
    // extension window).
    let half_signed: i64 = if half & 0x800000 != 0 {
        half as i64 - 0x1000000
    } else {
        half as i64
    };

    Ok((half_signed * 2, hw1 & 0x1000 != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bl_encodes_fixed_opcode_bits() {
        let mut insn = [0u8; 4];
        encode_bl(&mut insn, 0).unwrap();
        let hw0 = u16::from_le_bytes([insn[0], insn[1]]);
        let hw1 = u16::from_le_bytes([insn[2], insn[3]]);
        assert_eq!((hw0 >> 11) & 0x1F, 0b11110, "first halfword 11110 prefix");
        assert_eq!(hw1 & 0x1000, 0x1000, "BL keeps the L (link) bit set");
        assert_eq!((hw1 >> 14) & 0b11, 0b11, "second halfword 11 prefix");
    }

    #[test]
    fn bw_preserves_clear_link_bit() {
        // A B.W site (L=0) re-encoded with link=None must stay B.W.
        let site = [0xF0u8, 0x00, 0x90, 0x0F]; // B.W-shaped second halfword
        let mut insn = site;
        encode_branch(&mut insn, 8, None).unwrap();
        let (_, l) = decode_branch(&insn).unwrap();
        assert!(!l, "link bit must be preserved (B.W stays B.W)");
    }

    #[test]
    fn bw_link_bit_never_set_by_jump24() {
        let mut insn = [0u8; 4];
        encode_branch(&mut insn, 4, Some(false)).unwrap();
        let (_, l) = decode_branch(&insn).unwrap();
        assert!(!l, "explicit B.W encoding must not set L");
    }

    #[test]
    fn roundtrip_positive_negative_and_zero() {
        let cases: [i64; 6] = [0, 4, 0x200, 0x7FE, -4, -0x100];
        for &off in &cases {
            for link in [Some(true), Some(false), None] {
                let mut insn = if link.is_none() {
                    // B.W-shaped site so the preserved L bit is defined
                    [0xF0u8, 0x00, 0x90, 0x0F]
                } else {
                    [0u8; 4]
                };
                encode_branch(&mut insn, off, link).unwrap();
                let (decoded, l) = decode_branch(&insn).unwrap();
                assert_eq!(decoded, off, "offset round-trip for {off}");
                assert_eq!(
                    l,
                    link.unwrap_or(false),
                    "link round-trip for {off} link={link:?}"
                );
            }
        }
    }

    #[test]
    fn out_of_range_and_misaligned_rejected() {
        let mut insn = [0u8; 4];
        assert!(encode_bl(&mut insn, 0x0100_0000).is_err(), "+16MB+1");
        assert!(encode_bl(&mut insn, -0x0100_0000 - 2).is_err(), "-16MB-2");
        assert!(encode_bl(&mut insn, 1).is_err(), "misaligned");
        assert!(encode_bl(&mut insn[..2], 4).is_err(), "short buffer");
    }
}
