//! Import relocation applier for RISC-V RV32 targets.
//!
//! Supported relocation kinds (`.lmod` reloc format):
//!
//! | kind | name | formula | size | notes |
//! |---|---|---|---|---|
//! | 8 | `R_RISCV_32` | S + A | 4 bytes | absolute 32-bit |
//! | 9 | `R_RISCV_CALL` | S + A - P | 4 bytes | JAL (R_RISCV_JAL + R_RISCV_CALL merged) |

use lmod::reloc::RelocKind;

pub const E_RELOC_UNSUPPORTED: u32 = 5204;

/// Apply one RISC-V import relocation.
pub fn apply_import_reloc(
    buf: &mut [u8],
    site_off: usize,
    kind: u8,
    sym_addr: u64,
    addend: i64,
) -> Result<(), u32> {
    let site_addr = (buf.as_ptr() as u64).wrapping_add(site_off as u64);
    match kind {
        k if k == RelocKind::RiscV32 as u8 => {
            // R_RISCV_32: S + A (write 4 bytes LE)
            if site_off + 4 > buf.len() {
                return Err(E_RELOC_UNSUPPORTED);
            }
            let val = sym_addr.wrapping_add(addend as u64) as u32;
            buf[site_off..site_off + 4].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        k if k == RelocKind::RiscVCall as u8 => {
            // R_RISCV_CALL: JAL instruction.
            // Offset = S + A - P, encoded as 21-bit signed immediate in JAL.
            if site_off + 4 > buf.len() {
                return Err(E_RELOC_UNSUPPORTED);
            }
            let offset = (sym_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(site_addr as i64);
            // For RISC-V JAL, PC is the instruction address (not +4).
            let adj_offset = offset;
            encode_riscv_jal(&mut buf[site_off..site_off + 4], adj_offset)
                .map_err(|_| E_RELOC_UNSUPPORTED)?;
            Ok(())
        }
        _ => Err(E_RELOC_UNSUPPORTED),
    }
}

/// Encode a JAL (jump-and-link) instruction at `insn` with a signed
/// 21-bit offset (in bytes, must be even).  Range: ±1 MiB.
fn encode_riscv_jal(insn: &mut [u8], offset: i64) -> Result<(), ()> {
    if offset & 1 != 0 {
        return Err(()); // misaligned
    }
    let imm = offset >> 1; // convert bytes to 2-byte halfwords
    if imm > 0xFFFFF || imm < -0x100000 {
        return Err(()); // out of range (±1 MiB)
    }
    let u = imm as u32;

    // JAL encoding: imm[20|10:1|11|19:12] | rd(5) | opcode(0x6F)
    // We preserve the existing rd field (bits 11:7) and opcode (bits 6:0).
    let existing = u32::from_le_bytes(insn[..4].try_into().unwrap());
    let rd = existing & 0x0F80; // preserve rd
    let opcode = existing & 0x7F; // preserve opcode

    let imm20 = (u >> 19) & 1;
    let imm10_1 = (u >> 8) & 0x3FF;
    let imm11 = (u >> 9) & 1;
    let imm19_12 = u & 0xFF;

    let enc = (imm20 << 31)
        | (imm10_1 << 21)
        | (imm11 << 20)
        | (imm19_12 << 12)
        | rd
        | opcode;

    insn[..4].copy_from_slice(&enc.to_le_bytes());
    Ok(())
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
}
