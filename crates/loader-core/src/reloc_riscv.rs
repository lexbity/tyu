//! Import relocation applier for RISC-V RV32 targets.
//!
//! Supported relocation kinds (`.lmod` reloc format):
//!
//! | kind | name | formula | size | notes |
//! |---|---|---|---|---|
//! | 8 | `R_RISCV_32` | S + A | 4 bytes | absolute 32-bit |
//! | 9 | `R_RISCV_CALL` | S + A - P | 4 bytes | JAL (R_RISCV_JAL + R_RISCV_CALL merged) |
use crate::error::LoadError;

use lmod::reloc::RelocKind;

pub use crate::error::E_RELOC_UNSUPPORTED;

/// Apply one RISC-V import relocation.
pub fn apply_import_reloc(
    buf: &mut [u8],
    site_off: usize,
    kind: u8,
    sym_addr: u64,
    addend: i64,
) -> Result<(), LoadError> {
    let site_addr = (buf.as_ptr() as u64).wrapping_add(site_off as u64);
    match kind {
        k if k == RelocKind::RiscV32 as u8 => {
            // R_RISCV_32: S + A (write 4 bytes LE)
            if site_off + 4 > buf.len() {
                return Err(LoadError::RelocUnsupported);
            }
            let val = sym_addr.wrapping_add(addend as u64) as u32;
            buf[site_off..site_off + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        k if k == RelocKind::RiscVCall as u8 => {
            // R_RISCV_CALL: JAL instruction.
            // Offset = S + A - P, encoded as 21-bit signed immediate in JAL.
            if site_off + 4 > buf.len() {
                return Err(LoadError::RelocUnsupported);
            }
            let offset = (sym_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(site_addr as i64);
            // For RISC-V JAL, PC is the instruction address (not +4).
            let adj_offset = offset;
            encode_riscv_jal(&mut buf[site_off..site_off + 4], adj_offset)
                .map_err(|_| LoadError::RelocUnsupported)?;
            Ok(())
        }
        _ => Err(LoadError::RelocUnsupported),
    }
}

/// Encode a JAL (jump-and-link) instruction at `insn` with a signed
/// 21-bit offset (in bytes, must be even).  Range: ±1 MiB.
fn encode_riscv_jal(insn: &mut [u8], offset: i64) -> Result<(), ()> {
    if offset & 1 != 0 {
        return Err(()); // misaligned
    }
    // JAL byte offset is a 21-bit signed value (±1 MiB); bit 0 is implicitly 0.
    if !(-0x10_0000..=0xF_FFFE).contains(&offset) {
        return Err(()); // out of range (±1 MiB)
    }

    // The immediate fields carry bits of the *byte* offset directly (NOT offset/2):
    //   inst[31]=off[20], inst[30:21]=off[10:1], inst[20]=off[11], inst[19:12]=off[19:12].
    let u = offset as u32;
    // Preserve the existing rd field (bits 11:7) and opcode (bits 6:0).
    let rd_opcode = u32::from_le_bytes(insn[..4].try_into().unwrap()) & 0xFFF;

    let imm20 = (u >> 20) & 1;
    let imm10_1 = (u >> 1) & 0x3FF;
    let imm11 = (u >> 11) & 1;
    let imm19_12 = (u >> 12) & 0xFF;

    let enc = (imm20 << 31) | (imm10_1 << 21) | (imm11 << 20) | (imm19_12 << 12) | rd_opcode;
    insn[..4].copy_from_slice(&enc.to_le_bytes());
    Ok(())
}

/// Decode a RISC-V JAL instruction back to its byte offset.
fn decode_riscv_jal(insn: &[u8]) -> Result<i64, ()> {
    if insn.len() < 4 {
        return Err(());
    }
    let u = u32::from_le_bytes(insn[..4].try_into().unwrap());
    if u & 0x7F != 0x6F {
        return Err(());
    } // not JAL

    let imm20 = (u >> 31) & 1;
    let imm10_1 = (u >> 21) & 0x3FF;
    let imm11 = (u >> 20) & 1;
    let imm19_12 = (u >> 12) & 0xFF;

    // Reassemble the byte offset directly (bit 0 is always 0).
    let off = (imm20 << 20) | (imm19_12 << 12) | (imm11 << 11) | (imm10_1 << 1);
    // Sign-extend from bit 20 (the top bit of the 21-bit offset).
    let off = if off & 0x100000 != 0 {
        off | 0xFFE00000
    } else {
        off
    };
    Ok((off as i32) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn riscv32_abs32_basic() {
        let mut buf = vec![0u8; 8];
        apply_import_reloc(&mut buf, 0, 8, 0x12345678, 0).unwrap();
        assert_eq!(u32::from_le_bytes(buf[..4].try_into().unwrap()), 0x12345678);
    }

    #[test]
    fn riscv32_abs32_with_addend() {
        let mut buf = vec![0u8; 8];
        apply_import_reloc(&mut buf, 0, 8, 0x1000, 0x234).unwrap();
        assert_eq!(u32::from_le_bytes(buf[..4].try_into().unwrap()), 0x1234);
    }

    #[test]
    fn riscv32_call_forward() {
        // JAL at offset 0, target at offset +0x200 (forward call)
        let mut buf = vec![0u8; 8];
        // JAL with rd=ra(x1), opcode=0x6F → 0x6F = 0b1101111 → LE = [0x6F, 0, 0, 0]
        buf[0..4].copy_from_slice(&[0x6F, 0, 0, 0]);
        let p = buf.as_ptr() as u64;
        apply_import_reloc(&mut buf, 0, 9, p + 0x200, 0).unwrap();
        let enc = u32::from_le_bytes(buf[..4].try_into().unwrap());
        assert_eq!(enc & 0x7F, 0x6F, "opcode preserved");
        assert_ne!(enc, 0x6F, "should be modified");
    }

    #[test]
    fn riscv32_call_out_of_range() {
        let mut buf = vec![0u8; 8];
        buf[0..4].copy_from_slice(&[0x6F, 0, 0, 0]);
        let p = buf.as_ptr() as u64;
        // Target far away (2 MiB) → out of range
        let result = apply_import_reloc(&mut buf, 0, 9, p + 0x200000, 0);
        assert!(result.is_err());
    }

    #[test]
    fn riscv32_unsupported_kind_rejected() {
        let mut buf = vec![0u8; 8];
        assert_eq!(
            apply_import_reloc(&mut buf, 0, 99, 0, 0).unwrap_err(),
            E_RELOC_UNSUPPORTED
        );
    }

    #[test]
    fn riscv_jal_encodes_known_patterns() {
        // offset 0 → all imm fields are zero; only opcode and rd remain.
        let mut insn = [0x6Fu8, 0, 0, 0];
        encode_riscv_jal(&mut insn, 0).unwrap();
        let val = u32::from_le_bytes(insn);
        assert_eq!(
            val & 0x7F,
            0x6F,
            "JAL opcode must be preserved for offset 0"
        );
        assert_eq!(val & 0xFFFFF80, 0, "offset 0 must have zero imm fields");
        // offset 4 → imm fields become non-zero
        let mut insn = [0x6Fu8, 0, 0, 0];
        encode_riscv_jal(&mut insn, 4).unwrap();
        let val = u32::from_le_bytes(insn);
        assert_eq!(
            val & 0x7F,
            0x6F,
            "JAL opcode must be preserved for offset 4"
        );
        assert_ne!(val & 0xFFFFF80, 0, "offset 4 must have non-zero imm fields");
    }

    #[test]
    fn riscv_jal_matches_hardware_encoding() {
        // Exact machine code per the RISC-V ISA (verified against riscv32-elf-as):
        // `jal x0, 16` == 0x0100006f, `jal x0, -28` == 0xfe5ff06f. A self-consistent
        // but ISA-incorrect encoder (the prior `offset >> 1` bug) would fail these.
        let mut insn = [0x6Fu8, 0, 0, 0];
        encode_riscv_jal(&mut insn, 16).unwrap();
        assert_eq!(u32::from_le_bytes(insn), 0x0100_006f, "jal x0, 16");

        let mut insn = [0x6Fu8, 0, 0, 0];
        encode_riscv_jal(&mut insn, -28).unwrap();
        assert_eq!(u32::from_le_bytes(insn), 0xfe5f_f06f, "jal x0, -28");
    }

    #[test]
    fn riscv_jal_encode_decode_roundtrip() {
        // Direct encode→decode round-trip via encode_riscv_jal / decode_riscv_jal.
        // Also tests rd-preservation by setting rd=x1 (ra).
        let cases: [i64; 7] = [4, -4, 0, 0x200, -0x200, 0x10000, -0x10000];
        for &off in &cases {
            let mut insn = [0xEFu8, 0, 0, 0]; // JAL with rd=ra(1) = 0x6F | (1<<7) = 0xEF
            assert!(
                encode_riscv_jal(&mut insn, off).is_ok(),
                "encode_riscv_jal({off:#x}) must succeed"
            );
            let decoded = decode_riscv_jal(&insn).unwrap_or(i64::MIN);
            assert_eq!(
                decoded, off,
                "JAL encode→decode round-trip failed for offset {off:#x}, got {decoded:#x}"
            );
            // Verify rd preserved.
            let enc = u32::from_le_bytes(insn);
            assert_eq!(
                (enc >> 7) & 0x1F,
                1,
                "JAL rd=ra must be preserved at offset {off:#x}"
            );
        }
    }
}
