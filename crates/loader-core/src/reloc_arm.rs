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
use crate::error::LoadError;

use lmod::reloc::RelocKind;

/// Error returned for unsupported relocation kinds.
pub use crate::error::E_RELOC_UNSUPPORTED;

/// Apply one ARM Thumb import relocation.
///
/// # Parameters
///
/// * `buf`      — Mutable view of the loaded code section.
/// * `site_off` — Byte offset within `buf` where the patch goes.
/// * `kind`     — Relocation kind.
/// * `sym_addr` — Resolved absolute address of the imported symbol.
/// * `addend`   — Addend (typically -4 for PC-relative ARM relocs,
///   0 for absolute).
pub fn apply_import_reloc(
    buf: &mut [u8],
    site_off: usize,
    kind: u8,
    sym_addr: u64,
    addend: i64,
) -> Result<(), LoadError> {
    let site_addr = (buf.as_ptr() as u64).wrapping_add(site_off as u64);
    match kind {
        k if k == RelocKind::ArmAbs32 as u8 => {
            // R_ARM_ABS32: S + A  (write 4 bytes LE)
            if site_off + 4 > buf.len() {
                return Err(LoadError::RelocUnsupported);
            }
            let val = sym_addr.wrapping_add(addend as u64) as u32;
            buf[site_off..site_off + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        k if k == RelocKind::ArmRel32 as u8 => {
            // R_ARM_REL32: S + A - P  (write 4 bytes LE)
            if site_off + 4 > buf.len() {
                return Err(LoadError::RelocUnsupported);
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
                return Err(LoadError::RelocUnsupported);
            }
            let branch_addr = sym_addr & !1;
            let offset = (branch_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(site_addr as i64);
            // For Thumb BL, PC is ahead by 4 bytes (instruction is 4 bytes).
            let adj_offset = offset - 4;
            encode_thumb_bl(&mut buf[site_off..site_off + 4], adj_offset)
                .map_err(|_| LoadError::RelocUnsupported)?;
            Ok(())
        }
        k if k == RelocKind::ArmThmJump24 as u8 => {
            // R_ARM_THM_JUMP24: B/BL.W instruction.
            if site_off + 4 > buf.len() {
                return Err(LoadError::RelocUnsupported);
            }
            let branch_addr = sym_addr & !1;
            let offset = (branch_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(site_addr as i64);
            // For Thumb B/BL, PC is ahead by 4 bytes.
            let adj_offset = offset - 4;
            encode_thumb_bl(&mut buf[site_off..site_off + 4], adj_offset)
                .map_err(|_| LoadError::RelocUnsupported)?;
            Ok(())
        }
        _ => Err(LoadError::RelocUnsupported),
    }
}

/// Encode a signed byte offset into a 4-byte Thumb-2 BL instruction.
///
/// The shared bit arithmetic lives in `lmod::reloc::thumb` (module-format
/// §3.1) so the packer's internal pre-resolution and this loader-side import
/// binding patch identical instruction shapes; the loader always writes the
/// BL (link) form. Encode/decode round-trips are pinned by that module's
/// tests (the decode oracle is test-only there — the firmware build keeps
/// the 16 KiB device-loader text budget, G7).
fn encode_thumb_bl(insn: &mut [u8], offset: i64) -> Result<(), ()> {
    lmod::reloc::thumb::encode_bl(insn, offset)
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

    fn aligned_site(buf: &[u8]) -> usize {
        let base = buf.as_ptr() as usize;
        (4 - (base & 3)) & 3
    }

    #[test]
    fn thm_call_encodes_bl_instruction() {
        // Patch a BL instruction: sym = site + 0x200
        let mut buf = vec![0u8; 16];
        let site = aligned_site(&buf);
        let p = (buf.as_ptr() as u64) + site as u64;
        // The instruction will be patched to BL target = p + 0x200
        apply_import_reloc(&mut buf, site, 5, p + 0x200, 0).unwrap();

        // Read back the two halfwords
        let hw0 = u16::from_le_bytes(buf[site..site + 2].try_into().unwrap());
        let hw1 = u16::from_le_bytes(buf[site + 2..site + 4].try_into().unwrap());

        // Verify it's a valid BL instruction (first hw bits 15-11 = 11110).
        assert_eq!(
            (hw0 >> 11) & 0x1F,
            0b11110,
            "not a BL instruction (hw0 top)"
        );

        // Verify it's a BL (not B) by checking bit 14 of hw1 = 1
        assert_eq!((hw1 >> 14) & 1, 1, "not a BL instruction (hw1 bit14)");

        // Verify some bits are set (the offset encoding is present)
        assert!(hw1 & 0x7FF != 0, "imm11 is zero - no offset encoded");
    }

    #[test]
    fn thm_call_zero_offset() {
        // BL target = site (offset 0)
        let mut buf = vec![0u8; 16];
        let site = aligned_site(&buf);
        let p = (buf.as_ptr() as u64) + site as u64;
        apply_import_reloc(&mut buf, site, 5, p, 0).unwrap();

        // For BL target = site: offset = -4 (PC adjustment), half = -2
        // S = 1 (negative), J1 = ...
        // The instruction should still be valid
        let hw0 = u16::from_le_bytes(buf[site..site + 2].try_into().unwrap());
        assert_eq!((hw0 >> 11) & 0x1F, 0b11110);
    }

    #[test]
    fn thm_call_negative_offset() {
        let mut buf = vec![0u8; 16];
        let site = aligned_site(&buf);
        let p = (buf.as_ptr() as u64) + site as u64;
        // Target is before the site
        apply_import_reloc(&mut buf, site, 5, p.wrapping_sub(0x100), 0).unwrap();
        let hw0 = u16::from_le_bytes(buf[site..site + 2].try_into().unwrap());
        assert_eq!((hw0 >> 11) & 0x1F, 0b11110);
    }

    #[test]
    fn thm_jump24_encodes_branch() {
        let mut buf = vec![0u8; 16];
        let site = aligned_site(&buf);
        let p = (buf.as_ptr() as u64) + site as u64;
        apply_import_reloc(&mut buf, site, 6, p + 0x200, 0).unwrap();
        let hw0 = u16::from_le_bytes(buf[site..site + 2].try_into().unwrap());
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
    fn thm_call_accepts_thumb_bit_function_pointer() {
        let mut buf = vec![0u8; 16];
        let site = aligned_site(&buf);
        let p = (buf.as_ptr() as u64) + site as u64;
        apply_import_reloc(&mut buf, site, 5, p + 1, 0).unwrap();
    }

    #[test]
    fn thumb_bl_encodes_known_patterns() {
        // Verify encode_thumb_bl produces the correct bit patterns.
        // offset = 0 → both halfwords zero except fixed bits
        let mut insn = [0u8; 4];
        encode_thumb_bl(&mut insn, 0).unwrap();
        let hw0 = u16::from_le_bytes([insn[0], insn[1]]);
        let hw1 = u16::from_le_bytes([insn[2], insn[3]]);
        // hw0 bits 15:12 = 1111, bit 11 = 1, bits 9:8 = 11, bit 5 = 1
        // Note: bits 9:8 are not part of the immediate — they are fixed
        // opcode bits for the BL instruction.
        assert_ne!(hw0 & 0xF800, 0, "hw0 fixed bits");
        assert_ne!(hw1 & 0xF800, 0, "hw1 fixed bits");
        // offset = 4: hw = 2, imm11 = 2, rest zero
        let mut insn = [0u8; 4];
        encode_thumb_bl(&mut insn, 4).unwrap();
        let hw1 = u16::from_le_bytes([insn[2], insn[3]]);
        assert_eq!(hw1 & 0x7FF, 2, "imm11 for offset 4 must be 2");
    }

    #[test]
    fn thumb_bl_out_of_range_rejected() {
        let mut insn = [0u8; 4];
        assert!(
            encode_thumb_bl(&mut insn, 0x0100_0000).is_err(),
            "BL offset +16MB+1 must be rejected"
        );
        assert!(
            encode_thumb_bl(&mut insn, -0x0100_0000 - 2).is_err(),
            "BL offset -16MB-2 must be rejected"
        );
    }

    #[test]
    fn thumb_bl_misaligned_rejected() {
        let mut insn = [0u8; 4];
        assert!(
            encode_thumb_bl(&mut insn, 1).is_err(),
            "BL misaligned offset must be rejected"
        );
    }
}
