//! TrustLevel-Two `stack_bound` re-derivation (architecture-specific scanners).
//!
//! Re-runs a data-stack depth analysis over the verified code section bytes
//! to produce a conservative upper bound on stack usage.  The analysis is
//! necessarily conservative — it may over-estimate but will not under-estimate.
//!
//! Canonical reference: stack-bound-analysis.md; engineering-spec §3.
//!
//! # Direction convention
//!
//! The data stack grows **upward** on every architecture: a push increments the
//! DS register (`r15` on x86_64, `r4` on ARM Thumb, `s2` on RISC-V) and a pop
//! decrements it.  The running offset `off` tracks `DS_ptr − base_ptr` in bytes:
//! positive after a push, negative after a pop.
//!
//! Peak is raised **only** on the push (positive) direction — a pop never
//! contributes to high-water.

/// ⊤ sentinel — "no finite bound provable" (abi-contract §2.2).
pub const TOP_SENTINEL: u32 = 0xFFFF_FFFF;

/// Which target architecture to scan for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arch {
    /// x86_64 — data-stack pointer is `r15`.
    /// Patterns: `49 83 c7 XX` (add r15, imm8 — push),
    ///           `49 83 ef XX` (sub r15, imm8 — pop).
    X86_64,
    /// ARM Thumb (Cortex-M) — data-stack pointer is `r4`.
    /// Patterns: 16-bit `ADDS r4, r4, #N` / `SUBS r4, r4, #N`.
    ArmThumb,
    /// RISC-V (RV32) — data-stack pointer is `s2` (x18).
    /// Patterns: `addi s2, s2, +N` (push) / `addi s2, s2, -N` (pop).
    RiscV,
}

impl Arch {
    /// Detect architecture from relocation-table contents.
    /// If any ARM-family relocation kind is present, returns `ArmThumb`.
    /// Otherwise defaults to `X86_64`.
    pub fn detect_from_code(code: &[u8]) -> Self {
        // Check for distinctive 16-bit Thumb ADD/SUB immediate patterns.
        // These have bits 15:11 = 00011 (top 5 bits = 3).
        // For ADDS/SUBS R4,R4: w = 0x1C24 / 0x1E44 (LE).
        if code.len() >= 2 {
            for i in 0..code.len().saturating_sub(1) {
                let w = u16::from_le_bytes([code[i], code[i + 1]]);
                if (w >> 11) == 0b00011 {
                    let rd = (w & 0x7) as u32;
                    let rn = ((w >> 4) & 0x7) as u32;
                    // The DS register is `r4` (abi-contract §4.4.2).
                    // `r7` is NOT the DS pointer — matching it produces spurious deltas.
                    if rd == rn && rd == 4 {
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

/// x86_64 scanner: tracks `add r15, imm8` (push) and `sub r15, imm8` (pop).
///
/// Push (`add r15, N`) increments the DS pointer → positive offset → raises peak.
/// Pop  (`sub r15, N`) decrements the DS pointer → offset moves toward / below base.
///
/// Returns `TOP_SENTINEL` for patterns the scanner cannot safely bound:
/// - `add r15, imm32` (`49 81 c7 XX XX XX XX`) — recognized DS growth, not counted.
/// - Any `REX`-prefixed instruction modifying register `r15` that is not a
///   recognized push/pop — treat as unverifiable.
/// - Backward jump (loop) — static analysis cannot bound loop iterations.
fn rederive_x86_64(code: &[u8], slot_bytes: u32) -> u32 {
    let mut off: i64 = 0; // bytes above base (push) or below (pop)
    let mut peak: u32 = 0;
    let mut i = 0;
    while i < code.len() {
        let b = code[i];

        // Backward jump → loop → cannot statically bound push count.
        if b == 0xEB && i + 2 <= code.len() {
            let rel = code[i + 1] as i8 as i64;
            if rel < 0 {
                return TOP_SENTINEL;
            }
        }
        if (0x70..=0x7F).contains(&b) && i + 2 <= code.len() {
            let rel = code[i + 1] as i8 as i64;
            if rel < 0 {
                return TOP_SENTINEL;
            }
        }

        // add r15, imm8 — PUSH (DS grows upward: 49 83 c7 XX)
        if i + 3 < code.len() && b == 0x49 && code[i + 1] == 0x83 && code[i + 2] == 0xc7 {
            let imm = code[i + 3] as i8 as i64;
            off += imm;
            update_peak(&mut peak, off, slot_bytes);
            i += 4;
            continue;
        }
        // sub r15, imm8 — POP (DS shrinks: 49 83 ef XX)
        if i + 3 < code.len() && b == 0x49 && code[i + 1] == 0x83 && code[i + 2] == 0xef {
            let imm = code[i + 3] as i8 as i64;
            off -= imm;
            if off < 0 {
                off = 0;
            }
            i += 4;
            continue;
        }
        // add r15, imm32 (49 81 c7 XX XX XX XX) — recognized but not counted.
        if i + 6 < code.len() && b == 0x49 && code[i + 1] == 0x81 && code[i + 2] == 0xc7 {
            return TOP_SENTINEL;
        }
        // Any other REX-prefixed instruction targeting register 7 (r15)
        // that is not the recognized `49 83 c7` or `49 83 ef` sequence.
        // The ModRM byte is at position i+1 (after REX) for instructions
        // that have one; for instructions like `add r/m64, r64` (opcode 01)
        // the ModRM is at i+1 and the opcode selects the form.
        if (b & 0xEF) == 0x49 && i + 2 < code.len() {
            let opc = code[i + 1];
            let modrm = code[i + 2];
            let rm = modrm & 0x7;
            // Opcodes that target r15: 01 (add), 03 (sub), 09 (or), 11 (adc),
            // 13 (sbb), 19 (sbb), 21 (and), 23 (and), 29 (sub), 31 (xor),
            // 39 (cmp), 89 (mov), 8B (mov), 87 (xchg), 85 (test), etc.
            if rm == 7
                && (opc == 0x01
                    || opc == 0x03
                    || opc == 0x29
                    || opc == 0x89
                    || opc == 0x8B
                    || opc == 0x85
                    || opc == 0x09
                    || opc == 0x21
                    || opc == 0x31
                    || opc == 0x39)
            {
                return TOP_SENTINEL;
            }
        }
        i += 1;
    }
    peak
}

/// ARM Thumb scanner: tracks 16-bit `ADDS r4, r4, #N` (push) and
/// `SUBS r4, r4, #N` (pop).
///
/// Push (`ADDS`) increments the DS register `r4` → positive offset → raises peak.
/// Pop  (`SUBS`) decrements `r4` → offset moves toward / below base.
fn rederive_arm_thumb(code: &[u8], slot_bytes: u32) -> u32 {
    let mut off: i64 = 0;
    let mut peak: u32 = 0;
    let mut i = 0;

    while i + 1 < code.len() {
        let w = u16::from_le_bytes([code[i], code[i + 1]]);

        // 16-bit Thumb ADD/SUB immediate encoding:
        //   bits 15:11 = 00011 (fixed)
        //   bit 10 = op (1=SUB, 0=ADD)
        //   bits 9:7 = imm3 (0-7)
        //   bits 6:4 = Rn
        //   bits 2:0 = Rd
        //
        // The DS register is `r4` only (abi-contract §4.4.2).
        // `r7` is excluded — matching it produces spurious deltas
        // (false over- or under-count).
        if (w >> 11) == 0b00011 {
            let rd = (w & 0x7) as u32;
            let rn = ((w >> 4) & 0x7) as u32;
            let op_is_sub = ((w >> 10) & 1) as u32;
            if rd == rn && rd == 4 {
                let imm3 = ((w >> 7) & 0x7) as i64;
                if op_is_sub == 0 {
                    // ADD → push: DS grows upward
                    off += imm3;
                    update_peak(&mut peak, off, slot_bytes);
                } else {
                    // SUB → pop: DS shrinks
                    off -= imm3;
                    if off < 0 {
                        off = 0;
                    }
                }
                i += 2;
                continue;
            }
        }

        i += 1;
    }
    peak
}

/// RISC-V RV32 scanner: tracks `addi s2, s2, +N` (push, imm > 0) and
/// `addi s2, s2, -N` (pop, imm < 0) to compute DS peak.
///
/// Push (positive imm) increments the DS register `s2` → positive offset → raises peak.
/// Pop  (negative imm) decrements `s2` → offset moves toward / below base.
fn rederive_riscv(code: &[u8], slot_bytes: u32) -> u32 {
    let mut off: i64 = 0;
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
            let imm = (((imm12 as i32) << 20) >> 20) as i64;
            if imm > 0 {
                // push: DS grows upward
                off += imm;
                update_peak(&mut peak, off, slot_bytes);
            } else if imm < 0 {
                // pop: DS shrinks
                off += imm; // add negative = decrement
                if off < 0 {
                    off = 0;
                }
            }
            // imm == 0 is a no-op (addi s2, s2, 0)
            i += 4;
            continue;
        }
        i += 1;
    }
    peak
}

/// Raise `peak` when `off` is positive (DS above base).
/// The data stack grows upward — a push increases the DS register, making
/// `off` positive.  `off` is always in bytes; `slot_bytes` converts to slots.
fn update_peak(peak: &mut u32, off: i64, slot_bytes: u32) {
    if off > 0 && slot_bytes > 0 {
        let depth = (off / slot_bytes as i64) as u32;
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

    // ---- x86_64 encoders ----
    // add r15, imm8 = push (DS grows upward)
    fn encode_add_r15(imm: i8) -> Vec<u8> {
        vec![0x49, 0x83, 0xc7, imm as u8]
    }
    // sub r15, imm8 = pop (DS shrinks)
    fn encode_sub_r15(imm: i8) -> Vec<u8> {
        vec![0x49, 0x83, 0xef, imm as u8]
    }

    // ---- ARM Thumb encoders ----
    // ADDS R4, R4, #imm3 = push (DS grows upward)
    fn encode_thumb_add_r4(imm3: u8) -> Vec<u8> {
        let iii = imm3 & 0x7;
        let w: u16 = (0b0001_1 << 11)   // fixed pattern bits 15:11 = 00011
            | (0 << 10)                  // op = ADD
            | ((iii as u16) << 7)        // imm3
            | (4 << 4)                   // Rn = R4
            | 4; // Rd = R4
        w.to_le_bytes().to_vec()
    }
    // SUBS R4, R4, #imm3 = pop (DS shrinks)
    fn encode_thumb_sub_r4(imm3: u8) -> Vec<u8> {
        let iii = imm3 & 0x7;
        let w: u16 = (0b0001_1 << 11)
            | (1 << 10)                  // op = SUB
            | ((iii as u16) << 7)
            | (4 << 4)
            | 4;
        w.to_le_bytes().to_vec()
    }

    // ---- RISC-V encoder ----
    fn encode_riscv_addi_s2(imm: i32) -> Vec<u8> {
        let imm12 = imm as u32 & 0xfff;
        let insn = (imm12 << 20) | (18 << 15) | (0 << 12) | (18 << 7) | 0x13;
        let bytes = insn.to_le_bytes();
        let mut v = Vec::new();
        v.extend_from_slice(&bytes[..4]);
        v
    }

    // ===================================================================
    // x86_64 tests
    // ===================================================================

    #[test]
    fn x86_64_empty_code_zero_high() {
        assert_eq!(rederive_stack_high(b"", Arch::X86_64, 8), 0);
    }

    #[test]
    fn x86_64_single_push() {
        // add r15, 8 → 1 slot of DS growth
        let code = encode_add_r15(8);
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 1);
    }

    #[test]
    fn x86_64_push_then_pop() {
        // push 8 then pop 8 → peak should still be 1
        let mut code = encode_add_r15(8);
        code.extend_from_slice(&encode_sub_r15(8));
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 1);
    }

    #[test]
    fn x86_64_three_pushes() {
        // three pushes of 8 bytes each → 3 slots
        let mut code = Vec::new();
        for _ in 0..3 {
            code.extend_from_slice(&encode_add_r15(8));
        }
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 3);
    }

    #[test]
    fn x86_64_push_then_pop_peak_held() {
        // push 8, push 8, pop 8 → peak = 2 (held at max)
        let mut code = encode_add_r15(8);
        code.extend_from_slice(&encode_add_r15(8));
        code.extend_from_slice(&encode_sub_r15(8));
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 2);
    }

    #[test]
    fn x86_64_pops_only_zero_high() {
        // just pops (sub r15) with no prior push → peak = 0
        let mut code = Vec::new();
        for _ in 0..3 {
            code.extend_from_slice(&encode_sub_r15(8));
        }
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 0);
    }

    #[test]
    fn x86_64_mixed_push_pop_varied_imm() {
        // push 16, pop 8, push 8 → max offset 16, then 8, then 16 → peak = 2 (16/8)
        let mut code = encode_add_r15(16);
        code.extend_from_slice(&encode_sub_r15(8));
        code.extend_from_slice(&encode_add_r15(8));
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 2);
    }

    // ===================================================================
    // ARM Thumb tests
    // ===================================================================

    #[test]
    fn arm_thumb_empty_code() {
        assert_eq!(rederive_stack_high(b"", Arch::ArmThumb, 4), 0);
    }

    #[test]
    fn arm_thumb_single_push() {
        // adds r4, r4, #4 → 1 slot (slot_bytes = 4)
        let code = encode_thumb_add_r4(4);
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 1);
    }

    #[test]
    fn arm_thumb_push_then_pop() {
        let mut code = encode_thumb_add_r4(4);
        code.extend_from_slice(&encode_thumb_sub_r4(4));
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 1);
    }

    #[test]
    fn arm_thumb_three_pushes() {
        let mut code = Vec::new();
        for _ in 0..3 {
            code.extend_from_slice(&encode_thumb_add_r4(4));
        }
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 3);
    }

    #[test]
    fn arm_thumb_push_peak_held() {
        // push 4, push 4, pop 4 → peak = 2
        let mut code = encode_thumb_add_r4(4);
        code.extend_from_slice(&encode_thumb_add_r4(4));
        code.extend_from_slice(&encode_thumb_sub_r4(4));
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 2);
    }

    #[test]
    fn arm_thumb_pops_only_zero_high() {
        let mut code = Vec::new();
        for _ in 0..3 {
            code.extend_from_slice(&encode_thumb_sub_r4(4));
        }
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 0);
    }

    /// `r7` must NOT be detected as a DS register — only `r4` is the DS pointer.
    #[test]
    fn arm_thumb_r7_not_ds_register() {
        // SUBS R7, R7, #4: encoding uses Rd=Rn=7
        let iii = 4u8 & 0x7;
        let w: u16 = (0b0001_1 << 11) | (1 << 10) | ((iii as u16) << 7) | (7 << 4) | 7;
        let code = w.to_le_bytes().to_vec();
        // Should NOT match (r4 only) → peak = 0
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 0);
    }

    // ===================================================================
    // RISC-V tests
    // ===================================================================

    #[test]
    fn riscv_empty_code() {
        assert_eq!(rederive_stack_high(b"", Arch::RiscV, 4), 0);
    }

    #[test]
    fn riscv_single_push_4_bytes() {
        // addi s2, s2, 4 → 1 slot (slot_bytes = 4)
        let code = encode_riscv_addi_s2(4);
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 1);
    }

    #[test]
    fn riscv_push_then_pop() {
        let mut code = encode_riscv_addi_s2(4);
        code.extend_from_slice(&encode_riscv_addi_s2(-4));
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 1);
    }

    #[test]
    fn riscv_three_pushes() {
        let mut code = Vec::new();
        for _ in 0..3 {
            code.extend_from_slice(&encode_riscv_addi_s2(4));
        }
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 3);
    }

    #[test]
    fn riscv_push_peak_held() {
        // push 4, push 4, pop 8 → peak = 2
        let mut code = encode_riscv_addi_s2(4);
        code.extend_from_slice(&encode_riscv_addi_s2(4));
        code.extend_from_slice(&encode_riscv_addi_s2(-8));
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 2);
    }

    #[test]
    fn riscv_pops_only_zero_high() {
        let mut code = Vec::new();
        for _ in 0..3 {
            code.extend_from_slice(&encode_riscv_addi_s2(-8));
        }
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 0);
    }

    #[test]
    fn riscv_push_8_two_slots() {
        // addi s2, s2, 8 → 2 slots (8 bytes / 4 per slot)
        let code = encode_riscv_addi_s2(8);
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 2);
    }

    // ===================================================================
    // Arch detection tests
    // ===================================================================

    #[test]
    fn detect_x86_64_from_empty_code() {
        assert_eq!(Arch::detect_from_code(b""), Arch::X86_64);
    }

    #[test]
    fn detect_x86_64_from_x86_push() {
        let code = encode_add_r15(8);
        assert_eq!(Arch::detect_from_code(&code), Arch::X86_64);
    }

    #[test]
    fn detect_arm_thumb_from_thumb_code() {
        let code = encode_thumb_add_r4(4);
        assert_eq!(Arch::detect_from_code(&code), Arch::ArmThumb);
    }

    #[test]
    fn detect_riscv_from_code() {
        let code = encode_riscv_addi_s2(4);
        assert_eq!(Arch::detect_from_code(&code), Arch::RiscV);
    }

    // ===================================================================
    // TOP_SENTINEL tests
    // ===================================================================

    #[test]
    fn top_sentinel_defined() {
        assert_eq!(TOP_SENTINEL, 0xFFFF_FFFF);
    }

    // ===================================================================
    // Integration: direction correctness tests
    // ===================================================================

    /// A multi-push sequence on each arch must produce nonzero high.
    #[test]
    fn x86_64_push_sequence_nonzero() {
        let mut code = Vec::new();
        for _ in 0..5 {
            code.extend_from_slice(&encode_add_r15(8));
        }
        assert!(rederive_stack_high(&code, Arch::X86_64, 8) > 0);
    }

    #[test]
    fn arm_thumb_push_sequence_nonzero() {
        let mut code = Vec::new();
        for _ in 0..5 {
            code.extend_from_slice(&encode_thumb_add_r4(4));
        }
        assert!(rederive_stack_high(&code, Arch::ArmThumb, 4) > 0);
    }

    #[test]
    fn riscv_push_sequence_nonzero() {
        let mut code = Vec::new();
        for _ in 0..5 {
            code.extend_from_slice(&encode_riscv_addi_s2(4));
        }
        assert!(rederive_stack_high(&code, Arch::RiscV, 4) > 0);
    }

    /// Pops-only sequences must produce zero high (pops don't raise peak).
    #[test]
    fn x86_64_pops_only_zero() {
        let mut code = Vec::new();
        for _ in 0..5 {
            code.extend_from_slice(&encode_sub_r15(8));
        }
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), 0);
    }

    #[test]
    fn arm_thumb_pops_only_zero() {
        let mut code = Vec::new();
        for _ in 0..5 {
            code.extend_from_slice(&encode_thumb_sub_r4(4));
        }
        assert_eq!(rederive_stack_high(&code, Arch::ArmThumb, 4), 0);
    }

    #[test]
    fn riscv_pops_only_zero() {
        let mut code = Vec::new();
        for _ in 0..5 {
            code.extend_from_slice(&encode_riscv_addi_s2(-8));
        }
        assert_eq!(rederive_stack_high(&code, Arch::RiscV, 4), 0);
    }

    // -------------------------------------------------------------------
    // TOP_SENTINEL tests — patterns the scanner cannot safely bound.
    // -------------------------------------------------------------------

    #[test]
    fn loop_yields_top() {
        // Backward jmp over a push → static scan cannot bound iterations.
        let mut code = Vec::new();
        code.push(0x49);
        code.push(0x83);
        code.push(0xc7);
        code.push(8); // add r15, 8
        code.push(0xEB);
        code.push(-6i8 as u8); // jmp -6 (back to add)
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), TOP_SENTINEL);
    }

    #[test]
    fn backward_jcc_yields_top() {
        // je rel8 pointing backwards
        let mut code = Vec::new();
        code.push(0x49);
        code.push(0x83);
        code.push(0xc7);
        code.push(8);
        code.push(0x74);
        code.push(-6i8 as u8); // je -6
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), TOP_SENTINEL);
    }

    #[test]
    fn add_r15_imm32_yields_top() {
        // 49 81 c7 XX XX XX XX — add r15, imm32 (the imm8-only scanner misses this)
        let code = vec![0x49, 0x81, 0xc7, 0x00, 0x10, 0x00, 0x00];
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), TOP_SENTINEL);
    }

    #[test]
    fn unknown_ds_write_yields_top() {
        // 49 01 c7 — add r15, rdi (REX.W add r/m64, r64 targeting r15)
        // The scanner doesn't recognize this but it modifies r15 → unverifiable.
        let code = vec![0x49, 0x01, 0xc7];
        assert_eq!(rederive_stack_high(&code, Arch::X86_64, 8), TOP_SENTINEL);
    }
}
