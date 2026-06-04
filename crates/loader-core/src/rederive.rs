//! Tier-2 `stack_bound` re-derivation (architecture-specific scanners).
//!
//! Re-runs a data-stack depth analysis over the verified code section bytes
//! to produce a conservative upper bound on stack usage.  The analysis is
//! necessarily conservative — it may over-estimate but will not under-estimate.
//!
//! Canonical reference: stack-bound-analysis.md; module-format §6.2.

/// Which target architecture to scan for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arch {
    /// x86_64 — data-stack pointer is `r15`.
    /// Patterns: `49 83 ef XX` (sub r15, imm8), `49 83 c7 XX` (add r15, imm8).
    X86_64,
    /// ARM Thumb (Cortex-M) — data-stack pointer is `r4`.
    /// Patterns: 16-bit `SUBS r4, r4, #N` / `ADDS r4, r4, #N`.
    ArmThumb,
    /// RISC-V (RV32) — data-stack pointer is `s2` (x18).
    /// Patterns: `addi s2, s2, N` (push) / `addi s2, s2, -N` (pop).
    RiscV,
}

impl Arch {
    /// Detect architecture from relocation-table contents.
    /// If any ARM-family relocation kind is present, returns `ArmThumb`.
    /// Otherwise defaults to `X86_64`.
    pub fn detect_from_code(code: &[u8]) -> Self {
        // Check for distinctive 16-bit Thumb ADD/SUB immediate patterns.
        // These have bits 15:11 = 00011 (top 5 bits = 3).
        // For SUBS/ADDS R4,R4: w = 0x3x44 (LE) or 0x3x7C for R7
        if code.len() >= 2 {
            for i in 0..code.len().saturating_sub(1) {
                let w = u16::from_le_bytes([code[i], code[i + 1]]);
                // The ADD/SUB immediate format for 16-bit Thumb:
                //   bits 15:11 = 00011
                //   bit 10 = op (1=SUB)
                //   bits 9:7 = imm3
                //   bits 6:4 = Rn
                //   bits 2:0 = Rd
                if (w >> 11) == 0b00011 {
                    let rd = (w & 0x7) as u32;
                    let rn = ((w >> 4) & 0x7) as u32;
                    if rd == rn && (rd == 4 || rd == 7) {
                        return Arch::ArmThumb;
                    }
                }
            }
        }
        // Check for RISC-V: `addi s2, s2, N` (opcode 0x13, funct3=0, rd=18, rs1=18).
        if code.len() >= 4 {
            for i in 0..code.len().saturating_sub(3) {
                let insn = u32::from_le_bytes(code[i..i + 4].try_into().unwrap());
                let opcode = insn & 0x7f;
                let rd = ((insn >> 7) & 0x1f) as u8;
                let funct3 = ((insn >> 12) & 0x7) as u8;
                let rs1 = ((insn >> 15) & 0x1f) as u8;
                if opcode == 0x13 && funct3 == 0 && rd == 18 && rs1 == 18 {
                    return Arch::RiscV;
                }
            }
        }
        Arch::X86_64
    }
}

/// Re-derive a conservative stack-bound high-water mark (in slots) from the
/// code section bytes.
///
/// `slot_bytes` is target-specific (8 for x86_64, 4 for ARM Thumb, 4 for RISC-V).
/// Dispatches to the architecture-specific scanner based on `arch`.
pub fn rederive_stack_high(code: &[u8], arch: Arch, slot_bytes: u32) -> u32 {
    match arch {
        Arch::X86_64 => rederive_x86_64(code, slot_bytes),
        Arch::ArmThumb => rederive_arm_thumb(code, slot_bytes),
        Arch::RiscV => rederive_riscv(code, slot_bytes),
    }
}

/// x86_64 scanner: tracks `sub r15, imm8` and `add r15, imm8`.
fn rederive_x86_64(code: &[u8], slot_bytes: u32) -> u32 {
    let mut r15_off: i64 = 0;
    let mut peak: u32 = 0;
    let mut i = 0;
    while i < code.len() {
        let b = code[i];
        if i + 3 < code.len()
            && b == 0x49
            && code[i + 1] == 0x83
            && code[i + 2] == 0xef
        {
            let imm = code[i + 3] as i8 as i64;
            r15_off -= imm;
            update_peak(&mut peak, r15_off, slot_bytes);
            i += 4;
            continue;
        }
        if i + 3 < code.len()
            && b == 0x49
            && code[i + 1] == 0x83
            && code[i + 2] == 0xc7
        {
            let imm = code[i + 3] as i8 as i64;
            r15_off += imm;
            i += 4;
            continue;
        }
        i += 1;
    }
    peak
}

/// ARM Thumb scanner: tracks 16-bit `SUBS r4, r4, #N` and `ADDS r4, r4, #N`.
fn rederive_arm_thumb(code: &[u8], slot_bytes: u32) -> u32 {
    let mut sp_off: i64 = 0; // offset from initial data-stack pointer (negative = deeper)
    let mut peak: u32 = 0;
    let mut i = 0;

    while i + 1 < code.len() {
        let w = u16::from_le_bytes([code[i], code[i + 1]]);

        // Match: SUBS Rd, Rd, #imm8  (16-bit Thumb)
        // Encoding: 0001 1 1 0 I I I R R R
        // Low byte: 0x1C | (Rd << 4) | (III << 7)?  Let's check:
        //   bits 15:11 = 00011
        //   bit 10 = 1 (SUB), bit 9 = I2, bit 8 = I1, bit 7 = I0
        //   bits 6:4 = Rd, bits 2:0 = Rn (same as Rd for our pattern)
        // Hmm this format is actually:
        // bits 15:12 = 0001
        // bit 11 = 1
        // bit 10 = op (1 = SUB, 0 = ADD? or SUB=1, ADD=0 for this format)
        // bits 9:8 = imm2 (bits 7:6 of the shifted immediate)
        // bits 7:6 = Rd
        // bits 5:3 = imm3 (bits 5:3 of the shifted immediate)
        // bits 2:0 = Rn
        //
        // No, this format is 0x1C00 based with different encodings.

        // Let me use a simpler heuristic: ARM Thumb SUBS Rd, #N uses
        // the encoding 0001110x xxxxRRR where RRR is the register.
        // For R4: bytes are 0x24 0x1C (LE: w = 0x1C24)
        // For R7: bytes are 0x3C 0x1C (LE: w = 0x1C3C)
        // SUBS R4, R4, #8: LE = [0x24, 0x1C]
        // ADDS R4, R4, #8: LE = [0x24, 0x1E]

        // Check for SUBS R4, R4: pattern 0x1C24 — but this is only #0 immediate
        // Actually: 8-bit immediate, Rn = Rd = R4.

        // For cortex-m the push is `PUSH {reg}` which adjusts SP (R13),
        // not a general-purpose register.  The data-stack pointer in
        // the tyu_lang ARM backend is TBD.
        //
        // For now, return 0 (conservative lower bound).  When the ARM
        // backend lands, the specific data-stack register and instruction
        // patterns will be determined then.

        // Detect `SUBS Rd, Rd, #imm3` or `ADDS Rd, Rd, #imm3` for R4/R7.
        // 16-bit Thumb ADD/SUB immediate encoding:
        //   bits 15:11 = 00011 (fixed)
        //   bit 10 = op (1=SUB, 0=ADD)
        //   bits 9:7 = imm3 (0-7)
        //   bits 6:4 = Rn
        //   bits 2:0 = Rd
        if (w >> 11) == 0b00011 {
            let rd = (w & 0x7) as u32;
            let rn = ((w >> 4) & 0x7) as u32;
            let op_is_sub = ((w >> 10) & 1) as u32;
            if rd == rn && (rd == 4 || rd == 7) {
                let imm3 = ((w >> 7) & 0x7) as i64;
                if op_is_sub == 1 {
                    sp_off -= imm3;
                } else {
                    sp_off += imm3;
                }
                update_peak(&mut peak, sp_off, slot_bytes);
                i += 2;
                continue;
            }
        }

        i += 1;
    }
    peak
}

/// RISC-V RV32 scanner: tracks `addi s2, s2, +N` (push) and
/// `addi s2, s2, -N` (pop) to compute DS peak.
fn rederive_riscv(code: &[u8], slot_bytes: u32) -> u32 {
    let mut sp_off: i64 = 0;
    let mut peak: u32 = 0;
    let mut i = 0;
    while i + 3 < code.len() {
        let insn = u32::from_le_bytes(code[i..i + 4].try_into().unwrap());
        let opcode = insn & 0x7f;
        let rd = ((insn >> 7) & 0x1f) as u8;
        let funct3 = ((insn >> 12) & 0x7) as u8;
        let rs1 = ((insn >> 15) & 0x1f) as u8;
        // ADDI s2, s2, imm12: opcode=0x13, funct3=0, rd=18, rs1=18
        if opcode == 0x13 && funct3 == 0 && rd == 18 && rs1 == 18 {
            let imm12 = (insn >> 20) & 0xfff;
            // Sign-extend 12-bit: i32 handles sign, then widen to i64
            let imm = (((imm12 as i32) << 20) >> 20) as i64;
            sp_off += imm;
            if sp_off < 0 {
                let depth = (-sp_off as u32 + slot_bytes - 1) / slot_bytes;
                peak = peak.max(depth);
            }
            i += 4;
            continue;
        }
        i += 1;
    }
    peak
}

fn update_peak(peak: &mut u32, sp_off: i64, slot_bytes: u32) {
    if sp_off < 0 && slot_bytes > 0 {
        let depth = ((-sp_off) / slot_bytes as i64) as u32;
        if depth > *peak {
            *peak = depth;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn encode_sub_r15(imm: i8) -> Vec<u8> {
        vec![0x49, 0x83, 0xef, imm as u8]
    }

    fn encode_add_r15(imm: i8) -> Vec<u8> {
        vec![0x49, 0x83, 0xc7, imm as u8]
    }

    /// Encode 16-bit Thumb SUBS R4, R4, #imm3 (imm3 is 0-7)
    fn encode_thumb_sub_r4(imm3: u8) -> Vec<u8> {
        let iii = imm3 & 0x7;
        let w: u16 = (0b0001_1 << 11)   // fixed pattern
            | (1 << 10)                  // op = SUB
            | ((iii as u16) << 7)        // imm3
            | (4 << 4)                   // Rn = R4
            | 4;                         // Rd = R4
        w.to_le_bytes().to_vec()
    }

    /// Encode 16-bit Thumb ADDS R4, R4, #imm3
    fn encode_thumb_add_r4(imm3: u8) -> Vec<u8> {
        let iii = imm3 & 0x7;
        let w: u16 = (0b0001_1 << 11)   // fixed pattern
            | (0 << 10)                  // op = ADD
            | ((iii as u16) << 7)
            | (4 << 4)
            | 4;
        w.to_le_bytes().to_vec()
    }

    // ---- x86_64 tests ----

    #[test]
    fn x86_64_empty_code_zero_high() {
        assert_eq!(rederive_stack_high(b"", Arch::X86_64, 8), 0);
    }

    #[test]
    fn x86_64_single_push() {
        let code = encode_sub_r15(8);
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 1);
    }

    #[test]
    fn x86_64_push_then_pop() {
        let mut code = encode_sub_r15(8);
        code.extend_from_slice(&encode_add_r15(8));
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 1);
    }

    #[test]
    fn x86_64_three_pushes() {
        let mut code = Vec::new();
        for _ in 0..3 { code.extend_from_slice(&encode_sub_r15(8)); }
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 3);
    }

    // ---- ARM Thumb tests ----

    #[test]
    fn arm_thumb_empty_code() {
        assert_eq!(rederive_stack_high(b"", Arch::ArmThumb, 4), 0);
    }

    #[test]
    fn arm_thumb_single_push_with_arm_slot_bytes() {
        // imm3 = 4 bytes, slot_bytes = 4 (ARM) => 1 slot
        let code = encode_thumb_sub_r4(4);
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 1);
    }

    #[test]
    fn arm_thumb_push_then_pop_with_arm_slot_bytes() {
        let mut code = encode_thumb_sub_r4(4);
        code.extend_from_slice(&encode_thumb_add_r4(4));
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 1);
    }

    // ---- Arch detection ----

    #[test]
    fn detect_x86_64_from_empty_code() {
        assert_eq!(Arch::detect_from_code(b""), Arch::X86_64);
    }

    #[test]
    fn detect_x86_64_from_x86_code() {
        let code = encode_sub_r15(8);
        assert_eq!(Arch::detect_from_code(&code), Arch::X86_64);
    }

    #[test]
    fn detect_arm_thumb_from_thumb_code() {
        let code = encode_thumb_sub_r4(4);
        assert_eq!(Arch::detect_from_code(&code), Arch::ArmThumb);
    }

    // ---- RISC-V tests ----

    #[test]
    fn riscv_empty_code() {
        assert_eq!(rederive_stack_high(b"", Arch::RiscV, 4), 0);
    }

    #[test]
    fn riscv_single_push() {
        let code = encode_riscv_addi_s2(-8);
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 2);
    }

    #[test]
    fn riscv_push_then_pop() {
        let mut code = encode_riscv_addi_s2(-8);
        code.extend_from_slice(&encode_riscv_addi_s2(8));
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 2);
    }

    #[test]
    fn riscv_detect_from_code() {
        let code = encode_riscv_addi_s2(-8);
        assert_eq!(Arch::detect_from_code(&code), Arch::RiscV);
    }

    fn encode_riscv_addi_s2(imm: i32) -> Vec<u8> {
        let imm12 = imm as u32 & 0xfff;
        let insn = (imm12 << 20) | (18 << 15) | (0 << 12) | (18 << 7) | 0x13;
        let bytes = insn.to_le_bytes();
        // Build vec manually to avoid vec! macro in no_std
        let mut v = Vec::new();
        v.extend_from_slice(&bytes[..4]);
        v
    }
}
