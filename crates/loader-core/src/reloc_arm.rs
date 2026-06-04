//! Import relocation applier for ARM Thumb targets (Cortex-M).
//!
//! Supported relocation kinds (module-format §3.1):
//!
//! | kind | name | formula | size |
//! |---|---|---|---|
//! | 4 | `R_ARM_ABS32` | S + A | 4 bytes |
//! | 5 | `R_ARM_THM_CALL` | S + A - P | 4 bytes (BL/BLX) |
//! | 6 | `R_ARM_THM_JUMP24` | S + A - P | 4 bytes (B/BL) |
//! | 7 | `R_ARM_REL32` | S + A - P | 4 bytes |
//!
//! Thumb-bit handling: function symbols must have LSB=1 when
//! written as code pointers.  The caller should set the Thumb
//! bit before passing `sym_addr`.

use lmod::reloc::RelocKind;

/// Error returned for unsupported relocation kinds.
pub const E_RELOC_UNSUPPORTED: u32 = 5204;

/// Apply one ARM Thumb import relocation.
///
/// # Parameters
///
/// * `buf`      — Mutable view of the loaded code section.
/// * `site_off` — Byte offset within `buf` where the patch goes.
/// * `kind`     — Relocation kind.
/// * `sym_addr` — Resolved absolute address of the imported symbol.
/// * `addend`   — Addend (typically -4 for PC-relative ARM relocs,
///                0 for absolute).
pub fn apply_import_reloc(
    buf: &mut [u8],
    site_off: usize,
    kind: u8,
    sym_addr: u64,
    addend: i64,
) -> Result<(), u32> {
    let site_addr = (buf.as_ptr() as u64).wrapping_add(site_off as u64);
    match kind {
        k if k == RelocKind::ArmAbs32 as u8 => {
            // R_ARM_ABS32: S + A  (write 4 bytes LE)
            if site_off + 4 > buf.len() {
                return Err(E_RELOC_UNSUPPORTED);
            }
            let val = sym_addr.wrapping_add(addend as u64) as u32;
            buf[site_off..site_off + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        k if k == RelocKind::ArmRel32 as u8 => {
            // R_ARM_REL32: S + A - P  (write 4 bytes LE)
            if site_off + 4 > buf.len() {
                return Err(E_RELOC_UNSUPPORTED);
            }
            let val = (sym_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(site_addr as i64) as u32;
            buf[site_off..site_off + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        k if k == RelocKind::ArmThmCall as u8 => {
            // R_ARM_THM_CALL: BL/BLX instruction.
            // Offset = S + A - P, 4-byte Thumb-2 BL/BLX.
            // The offset is encoded as a signed 25-bit value in halfwords.
            if site_off + 4 > buf.len() {
                return Err(E_RELOC_UNSUPPORTED);
            }
            let offset = (sym_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(site_addr as i64);
            // For Thumb BL, PC is ahead by 4 bytes (instruction is 4 bytes).
            let adj_offset = offset - 4;
            encode_thumb_bl(&mut buf[site_off..site_off + 4], adj_offset)
                .map_err(|_| E_RELOC_UNSUPPORTED)?;
            Ok(())
        }
        k if k == RelocKind::ArmThmJump24 as u8 => {
            // R_ARM_THM_JUMP24: B/BL.W instruction.
            if site_off + 4 > buf.len() {
                return Err(E_RELOC_UNSUPPORTED);
            }
            let offset = (sym_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(site_addr as i64);
            // For Thumb B/BL, PC is ahead by 4 bytes.
            let adj_offset = offset - 4;
            encode_thumb_bl(&mut buf[site_off..site_off + 4], adj_offset)
                .map_err(|_| E_RELOC_UNSUPPORTED)?;
            Ok(())
        }
        _ => Err(E_RELOC_UNSUPPORTED),
    }
}

/// Encode a signed byte offset into a 4-byte Thumb-2 BL/BLX instruction.
///
/// The `offset` is in bytes and must be halfword-aligned (even) and
/// within ±16 MB (fits in a signed 25-bit value).  The encoding stores
/// a signed 24-bit value in halfwords (offset >> 1) into the instruction.
///
/// Instruction format for BL (ARMv7-M):
///   First halfword (at site_off):
///     1111 0 S 11 1 J2 J1 0 imm10[9:0]
///   Second halfword (at site_off+2):
///     1111 1 1 1 1 1 1 imm11[10:0]
///
/// Where:
///   S = (offset_half >> 23) & 1          (bit 10 of first hw)
///   J1 = ((offset_half >> 22) & 1) ^ !S  (bit 13 of second hw)
///   J2 = ((offset_half >> 21) & 1) ^ !S  (bit 11 of first hw)
///   imm10 = (offset_half >> 12) & 0x3FF  (bits 9:0 of first hw)
///   imm11 = offset_half & 0x7FF          (bits 10:0 of second hw)
fn encode_thumb_bl(insn: &mut [u8], offset: i64) -> Result<(), ()> {
    if offset & 1 != 0 {
        return Err(()); // misaligned
    }
    let half = offset >> 1; // convert bytes to halfwords
    // Check signed 24-bit range (±16MB)
    if half > 0x7FFFFF || half < -0x800000 {
        return Err(());
    }

    let h = half as u32;
    let s: u16 = ((h >> 23) & 1) as u16;
    let not_s: u16 = 1 - s;
    let j1: u16 = (((h >> 22) & 1) as u16) ^ not_s;
    let j2: u16 = (((h >> 21) & 1) as u16) ^ not_s;
    let imm10: u16 = ((h >> 12) & 0x3FF) as u16;
    let imm11: u16 = (h & 0x7FF) as u16;

    // Build first halfword
    let hw0 = 0b1111_0000_0000_0000u16  // 0xF000
        | (0b10 << 12)   // 0x2000: fixed bits for BL = 11110
        | (s << 10)      // sign bit
        | (0b11 << 8)    // opcode = BL
        | (j2 << 7)
        | (j1 << 6)
        | (1 << 5)       // always 1 for BL
        | imm10;

    // Build second halfword
    let hw1 = 0b1111_1000_0000_0000u16  // 0xF800
        | (1 << 14)      // opcode high bit
        | (1 << 12)      // always 1 for BL
        | imm11;

    insn[0..2].copy_from_slice(&hw0.to_le_bytes());
    insn[2..4].copy_from_slice(&hw1.to_le_bytes());
    Ok(())
}

/// Apply the Thumb-bit (LSB=1) to a function address.
/// ARM Thumb function pointers must have bit 0 set.
pub fn thumb_address(addr: usize) -> usize {
    addr | 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // -----------------------------------------------------------------------
    // R_ARM_ABS32
    // -----------------------------------------------------------------------

    #[test]
    fn abs32_writes_four_bytes() {
        let mut buf = vec![0u8; 8];
        apply_import_reloc(&mut buf, 0, 4, 0x12345678, 0).unwrap();
        let val = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, 0x12345678);
    }

    #[test]
    fn abs32_with_addend() {
        let mut buf = vec![0u8; 8];
        apply_import_reloc(&mut buf, 0, 4, 0x1000, 0x234).unwrap();
        let val = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, 0x1234);
    }

    #[test]
    fn abs32_at_nonzero_offset() {
        let mut buf = vec![0u8; 16];
        apply_import_reloc(&mut buf, 12, 4, 0xABCD, 0).unwrap();
        let val = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        assert_eq!(val, 0xABCD);
    }

    // -----------------------------------------------------------------------
    // R_ARM_REL32
    // -----------------------------------------------------------------------

    #[test]
    fn rel32_forward() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        apply_import_reloc(&mut buf, 0, 7, p + 0x100, 0).unwrap();
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, 0x100);
    }

    #[test]
    fn rel32_backward() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        apply_import_reloc(&mut buf, 0, 7, p.wrapping_sub(0x100), 0).unwrap();
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, -0x100i32);
    }

    // -----------------------------------------------------------------------
    // R_ARM_THM_CALL / R_ARM_THM_JUMP24
    // -----------------------------------------------------------------------

    #[test]
    fn thm_call_encodes_bl_instruction() {
        // Patch a BL instruction: sym = site + 0x200
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        // The instruction will be patched to BL target = p + 0x200
        apply_import_reloc(&mut buf, 0, 5, p + 0x200, 0).unwrap();

        // Read back the two halfwords
        let hw0 = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        let hw1 = u16::from_le_bytes(buf[2..4].try_into().unwrap());

        // Verify it's a valid BL instruction (first hw bits 15-11 = 11110, bits 9-8 = 11)
        assert_eq!((hw0 >> 11) & 0x1F, 0b11110, "not a BL instruction (hw0 top)");
        // Check opcode bits 9-8 = 11 (BL)
        assert_eq!((hw0 >> 8) & 0x3, 0b11, "not a BL instruction (opcode)");

        // Verify it's a BL (not B) by checking bit 14 of hw1 = 1
        assert_eq!((hw1 >> 14) & 1, 1, "not a BL instruction (hw1 bit14)");

        // Verify some bits are set (the offset encoding is present)
        assert!(hw1 & 0x7FF != 0, "imm11 is zero - no offset encoded");
    }

    #[test]
    fn thm_call_zero_offset() {
        // BL target = site (offset 0)
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        apply_import_reloc(&mut buf, 0, 5, p, 0).unwrap();

        // For BL target = site: offset = -4 (PC adjustment), half = -2
        // S = 1 (negative), J1 = ... 
        // The instruction should still be valid
        let hw0 = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        assert_eq!((hw0 >> 11) & 0x1F, 0b11110);
    }

    #[test]
    fn thm_call_negative_offset() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        // Target is before the site
        apply_import_reloc(&mut buf, 0, 5, p.wrapping_sub(0x100), 0).unwrap();
        let hw0 = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        assert_eq!((hw0 >> 11) & 0x1F, 0b11110);
    }

    #[test]
    fn thm_jump24_encodes_branch() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        apply_import_reloc(&mut buf, 0, 6, p + 0x200, 0).unwrap();
        let hw0 = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        assert_eq!((hw0 >> 11) & 0x1F, 0b11110);
    }

    // -----------------------------------------------------------------------
    // Thumb-bit
    // -----------------------------------------------------------------------

    #[test]
    fn thumb_address_sets_lsb() {
        assert_eq!(thumb_address(0x1000), 0x1001);
        assert_eq!(thumb_address(0x1001), 0x1001); // idempotent
    }

    // -----------------------------------------------------------------------
    // Rejection tests
    // -----------------------------------------------------------------------

    #[test]
    fn unsupported_kind_rejected() {
        let mut buf = vec![0u8; 8];
        let err = apply_import_reloc(&mut buf, 0, 0, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }

    #[test]
    fn x86_64_reloc_rejected_on_arm() {
        let mut buf = vec![0u8; 8];
        let err = apply_import_reloc(&mut buf, 0, 1, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }

    #[test]
    fn site_out_of_range_abs32() {
        let mut buf = vec![0u8; 2];
        let err = apply_import_reloc(&mut buf, 0, 4, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }

    #[test]
    fn thm_call_misaligned_offset_rejected() {
        // Offsets must be halfword-aligned (even)
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        // offset = 1 (odd) → should fail encoding
        let err = apply_import_reloc(&mut buf, 0, 5, p + 1, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }
}
