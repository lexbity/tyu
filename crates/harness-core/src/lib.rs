//! Host-side harness logic shared by the toolchain driver (`tyu`) and
//! integration tests (`execution-tests`).
//!
//! Re-exports the canonical stack-bound scanner from `loader-core::rederive`
//! and provides the serial-protocol parser and ELF code-section scanner — all
//! as no_std pure-byte functions so they can be called from any context.

#![no_std]

pub use loader_core::rederive::{Arch, rederive_stack_high, TOP_SENTINEL};

// ---------------------------------------------------------------------------
// Serial completion-protocol parser
// ---------------------------------------------------------------------------

/// Parsed summary of serial/test output emitted by a running image.
///
/// Scans for the marker protocol:
/// - `F` (0x46): a test failure was signalled
/// - `S` (0x53) followed by `\n`: all suites completed
/// - `H` (0x48) followed by u32-le: runtime high-water measurement (slots)
#[derive(Debug, Default, Eq, PartialEq)]
pub struct OutputSummary {
    pub failures: usize,
    pub completed: bool,
    pub high_slots: u32,
}

/// Parse serial output from a test run into an `OutputSummary`.
pub fn parse_output(stdout: &[u8]) -> OutputSummary {
    let mut failures = 0usize;
    let mut completed = false;
    let mut high_slots = 0u32;
    let mut i = 0;
    while i < stdout.len() {
        match stdout[i] {
            b'F' => failures += 1,
            b'S' => {
                if stdout.get(i + 1) == Some(&b'\n') {
                    completed = true;
                }
            }
            b'H' => {
                if i + 4 < stdout.len() {
                    high_slots = u32::from_le_bytes(
                        stdout[i + 1..i + 5].try_into().unwrap(),
                    );
                }
            }
            _ => {}
        }
        i += 1;
    }
    OutputSummary {
        failures,
        completed,
        high_slots,
    }
}

// ---------------------------------------------------------------------------
// ELF code-section scanner
// ---------------------------------------------------------------------------

/// Re-derive a conservative data-stack high-water bound (in slots) from the
/// code section of an ELF (ELF64 or ELF32), given as raw bytes.
///
/// Walks the program headers, selects `PT_LOAD` segments with `PF_X`, and
/// runs the architecture-specific scanner on each.  Returns the maximum over
/// all executable segments.
pub fn rederive_elf_high(elf: &[u8], slot_bytes: u8) -> u32 {
    if elf.len() < 64 {
        return 0;
    }
    if &elf[0..4] != b"\x7fELF" {
        return 0;
    }

    let elf_class = elf[4]; // 1 = ELF32, 2 = ELF64

    let (e_phoff, e_phentsize, e_phnum) = if elf_class == 2 {
        if elf.len() < 0x3a { return 0; }
        let phoff = u64::from_le_bytes(elf[0x20..0x28].try_into().unwrap()) as usize;
        let phent = u16::from_le_bytes(elf[0x36..0x38].try_into().unwrap()) as usize;
        let phnum = u16::from_le_bytes(elf[0x38..0x3a].try_into().unwrap()) as usize;
        (phoff, phent, phnum)
    } else if elf_class == 1 {
        if elf.len() < 0x2e { return 0; }
        let phoff = u32::from_le_bytes(elf[0x1c..0x20].try_into().unwrap()) as usize;
        let phent = u16::from_le_bytes(elf[0x2a..0x2c].try_into().unwrap()) as usize;
        let phnum = u16::from_le_bytes(elf[0x2c..0x2e].try_into().unwrap()) as usize;
        (phoff, phent, phnum)
    } else {
        return 0;
    };

    let mut total_high = 0u32;

    for i in 0..e_phnum {
        let off = e_phoff + i * e_phentsize;
        let phdr_size = if elf_class == 2 { 56usize } else { 32usize };
        if off + phdr_size > elf.len() {
            break;
        }

        // p_type at offset 0, p_flags at offset 4
        let p_type = u32::from_le_bytes(elf[off..off + 4].try_into().unwrap());
        if p_type != 1 {
            continue; // not PT_LOAD
        }
        let p_flags = u32::from_le_bytes(elf[off + 4..off + 8].try_into().unwrap());
        if p_flags & 1 == 0 {
            continue; // no PF_X
        }

        let (p_offset, p_filesz) = if elf_class == 2 {
            let po = u64::from_le_bytes(elf[off + 8..off + 16].try_into().unwrap()) as usize;
            let sz = u64::from_le_bytes(elf[off + 32..off + 40].try_into().unwrap()) as usize;
            (po, sz)
        } else {
            let po = u32::from_le_bytes(elf[off + 12..off + 16].try_into().unwrap()) as usize;
            let sz = u32::from_le_bytes(elf[off + 16..off + 20].try_into().unwrap()) as usize;
            (po, sz)
        };

        if p_offset + p_filesz > elf.len() || p_filesz == 0 {
            continue;
        }

        let code = &elf[p_offset..p_offset + p_filesz];
        let arch = Arch::detect_from_code(code);
        let slot32 = slot_bytes as u32;
        let high = rederive_stack_high(code, arch, slot32);
        total_high = total_high.max(high);
    }

    total_high
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::vec;
    use alloc::vec::Vec;
    use super::*;

    // ===================================================================
    // parse_output tests
    // ===================================================================

    #[test]
    fn empty_output() {
        let s = parse_output(b"");
        assert_eq!(s.failures, 0);
        assert!(!s.completed);
        assert_eq!(s.high_slots, 0);
    }

    #[test]
    fn completion_marker() {
        let s = parse_output(b"S\n");
        assert!(s.completed);
        assert_eq!(s.failures, 0);
    }

    #[test]
    fn failure_marker() {
        let s = parse_output(b"F");
        assert_eq!(s.failures, 1);
        assert!(!s.completed);
    }

    #[test]
    fn multiple_failures() {
        let s = parse_output(b"FF");
        assert_eq!(s.failures, 2);
    }

    #[test]
    fn s_without_newline_not_completed() {
        let s = parse_output(b"S");
        assert!(!s.completed);
    }

    #[test]
    fn high_water_marker() {
        let mut buf = Vec::new();
        buf.push(b'H');
        buf.extend_from_slice(&42u32.to_le_bytes());
        let s = parse_output(&buf);
        assert_eq!(s.high_slots, 42);
    }

    #[test]
    fn truncated_high_water_ignored() {
        // Only H + 3 bytes (needs 4)
        let mut buf = vec![b'H', 0x01, 0x02, 0x03];
        let s = parse_output(&buf);
        assert_eq!(s.high_slots, 0, "truncated H must not be parsed");
    }

    #[test]
    fn all_markers_together() {
        // F, H+8u32-le, S\n
        let mut buf = vec![b'F'];
        buf.push(b'H');
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(b"S\n");
        let s = parse_output(&buf);
        assert_eq!(s.failures, 1);
        assert_eq!(s.high_slots, 8);
        assert!(s.completed);
    }

    // ===================================================================
    // rederive_elf_high tests
    // ===================================================================

    /// Build a minimal ELF64 with one PT_LOAD+PF_X segment containing `code`.
    /// Returns the ELF bytes.
    fn make_elf64(code: &[u8]) -> Vec<u8> {
        let phdr_size: usize = 56;
        let ehdr_size: usize = 64;
        let total = ehdr_size + phdr_size + code.len();
        let mut buf = vec![0u8; total];

        // ELF identification
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2; // ELF64
        buf[5] = 1; // little-endian
        buf[6] = 1; // ELF version

        // e_phoff (64-bit): offset 0x20, 8 bytes
        buf[0x20..0x28].copy_from_slice(&(ehdr_size as u64).to_le_bytes());
        // e_phentsize: offset 0x36, 2 bytes
        buf[0x36..0x38].copy_from_slice(&(phdr_size as u16).to_le_bytes());
        // e_phnum: offset 0x38, 2 bytes
        buf[0x38..0x3a].copy_from_slice(&(1u16).to_le_bytes());

        // Program header at offset ehdr_size
        let phoff = ehdr_size;
        // p_type = PT_LOAD (1)
        buf[phoff..phoff + 4].copy_from_slice(&1u32.to_le_bytes());
        // p_flags = PF_X | PF_R (5)
        buf[phoff + 4..phoff + 8].copy_from_slice(&5u32.to_le_bytes());
        // p_offset (8 bytes) = after phdr
        let code_off = (ehdr_size + phdr_size) as u64;
        buf[phoff + 8..phoff + 16].copy_from_slice(&code_off.to_le_bytes());
        // p_filesz (8 bytes)
        buf[phoff + 32..phoff + 40].copy_from_slice(&(code.len() as u64).to_le_bytes());

        // Code
        let code_off_usize = ehdr_size + phdr_size;
        buf[code_off_usize..code_off_usize + code.len()].copy_from_slice(code);

        buf
    }

    /// Build a minimal ELF32 with one PT_LOAD+PF_X segment containing `code`.
    fn make_elf32(code: &[u8]) -> Vec<u8> {
        let phdr_size: usize = 32;
        let ehdr_size: usize = 52; // ELF32 ehdr is 52 bytes
        let total = ehdr_size + phdr_size + code.len();
        let mut buf = vec![0u8; total];

        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 1; // ELF32
        buf[5] = 1;
        buf[6] = 1;

        // e_phoff (32-bit): offset 0x1c, 4 bytes
        buf[0x1c..0x20].copy_from_slice(&(ehdr_size as u32).to_le_bytes());
        // e_phentsize: offset 0x2a, 2 bytes
        buf[0x2a..0x2c].copy_from_slice(&(phdr_size as u16).to_le_bytes());
        // e_phnum: offset 0x2c, 2 bytes
        buf[0x2c..0x2e].copy_from_slice(&(1u16).to_le_bytes());

        let phoff = ehdr_size;
        buf[phoff..phoff + 4].copy_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
        buf[phoff + 4..phoff + 8].copy_from_slice(&5u32.to_le_bytes()); // p_flags = PF_X|PF_R
        // p_offset (4 bytes) = after phdr
        let code_off = (ehdr_size + phdr_size) as u32;
        buf[phoff + 12..phoff + 16].copy_from_slice(&code_off.to_le_bytes());
        // p_filesz (4 bytes)
        buf[phoff + 16..phoff + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());

        let code_off_usize = ehdr_size + phdr_size;
        buf[code_off_usize..code_off_usize + code.len()].copy_from_slice(code);

        buf
    }

    #[test]
    fn elf64_no_code_returns_zero() {
        assert_eq!(rederive_elf_high(&make_elf64(b""), 8), 0);
    }

    #[test]
    fn elf32_no_code_returns_zero() {
        assert_eq!(rederive_elf_high(&make_elf32(b""), 4), 0);
    }

    #[test]
    fn elf64_x86_push_detected() {
        // add r15, 8 (push, 1 slot of 8 bytes)
        let code = vec![0x49, 0x83, 0xc7, 0x08];
        let elf = make_elf64(&code);
        assert_eq!(rederive_elf_high(&elf, 8), 1);
    }

    #[test]
    fn elf32_arm_push_detected() {
        // adds r4, r4, #4 (push, 1 slot of 4 bytes)
        // 0x1A44 LE = [0x44, 0x1A]
        let code = vec![0x44, 0x1a];
        let elf = make_elf32(&code);
        assert_eq!(rederive_elf_high(&elf, 4), 1);
    }

    #[test]
    fn elf64_riscv_push_detected() {
        // addi s2, s2, 4 (push, 1 slot of 4 bytes)
        let code = vec![0x13, 0x09, 0x49, 0x00]; // addi s2, s2, 4
        let elf = make_elf64(&code);
        assert_eq!(rederive_elf_high(&elf, 4), 1);
    }

    #[test]
    fn not_an_elf() {
        assert_eq!(rederive_elf_high(b"not an elf", 8), 0);
    }

    #[test]
    fn too_short_elf() {
        assert_eq!(rederive_elf_high(b"\x7fELF", 8), 0);
    }

    #[test]
    fn non_executable_segment_ignored() {
        // An ELF with PF_R only (no PF_X) should give 0
        let code = vec![0x49, 0x83, 0xc7, 0x08];
        let mut elf = make_elf64(&code);
        let phoff = 64usize;
        elf[phoff + 4..phoff + 8].copy_from_slice(&4u32.to_le_bytes()); // PF_R only
        assert_eq!(rederive_elf_high(&elf, 8), 0);
    }

    #[test]
    fn multiple_segments_max_taken() {
        // Two PT_LOAD+PF_X segments; scan takes the max high-water
        let push1 = vec![0x49, 0x83, 0xc7, 0x08]; // 1 slot
        let push3 = vec![
            0x49, 0x83, 0xc7, 0x08,
            0x49, 0x83, 0xc7, 0x08,
            0x49, 0x83, 0xc7, 0x08,
        ]; // 3 slots

        let phdr_size: usize = 56;
        let ehdr_size: usize = 64;
        let phdrs_off = ehdr_size;
        let seg1_off = phdrs_off + 2 * phdr_size;
        let seg2_off = seg1_off + push1.len();
        let total = seg2_off + push3.len();
        let mut buf = vec![0u8; total];

        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = 2;
        buf[5] = 1;
        buf[6] = 1;

        buf[0x20..0x28].copy_from_slice(&(phdrs_off as u64).to_le_bytes());
        buf[0x36..0x38].copy_from_slice(&(phdr_size as u16).to_le_bytes());
        buf[0x38..0x3a].copy_from_slice(&(2u16).to_le_bytes());

        // PHDR 0: seg1
        let ph0 = phdrs_off;
        buf[ph0..ph0 + 4].copy_from_slice(&1u32.to_le_bytes());
        buf[ph0 + 4..ph0 + 8].copy_from_slice(&5u32.to_le_bytes());
        buf[ph0 + 8..ph0 + 16].copy_from_slice(&(seg1_off as u64).to_le_bytes());
        buf[ph0 + 32..ph0 + 40].copy_from_slice(&(push1.len() as u64).to_le_bytes());

        // PHDR 1: seg2
        let ph1 = phdrs_off + phdr_size;
        buf[ph1..ph1 + 4].copy_from_slice(&1u32.to_le_bytes());
        buf[ph1 + 4..ph1 + 8].copy_from_slice(&5u32.to_le_bytes());
        buf[ph1 + 8..ph1 + 16].copy_from_slice(&(seg2_off as u64).to_le_bytes());
        buf[ph1 + 32..ph1 + 40].copy_from_slice(&(push3.len() as u64).to_le_bytes());

        buf[seg1_off..seg1_off + push1.len()].copy_from_slice(&push1);
        buf[seg2_off..seg2_off + push3.len()].copy_from_slice(&push3);

        assert_eq!(rederive_elf_high(&buf, 8), 3);
    }
}
