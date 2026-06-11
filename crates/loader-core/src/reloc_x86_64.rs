//! Import relocation applier for the x86_64 target.
//!
//! Canonical definition: module-format-and-loading.md §3.1, abi-contract §4.
//!
//! The runtime loader calls `apply_import_reloc` for every entry in the
//! import reloc table after resolving the imported symbol's address.
//!
//! Supported kinds (abi-contract §4, x86_64 subset):
//!   - `R_X86_64_64`    (kind 1) — absolute 64-bit:        S + A
//!   - `R_X86_64_PC32`  (kind 2) — PC-relative 32-bit:     S + A - P
//!   - `R_X86_64_PLT32` (kind 3) — PLT-relative 32-bit:    S + A - P
//!
//! Any other kind is rejected with `E_RELOC_UNSUPPORTED (5204)`.
use crate::error::LoadError;

use lmod::reloc::RelocKind;

/// Error returned for unsupported relocation kinds.
pub use crate::error::E_RELOC_UNSUPPORTED;

/// Apply one import relocation.
///
/// # Parameters
///
/// * `buf`     — Mutable view of the loaded image (code section data).
/// * `site_off` — Byte offset within `buf` where the patch goes.
/// * `kind`     — Relocation kind (`RelocKind::X86_64_64` = 1, etc.).
/// * `sym_addr` — Resolved absolute address of the imported symbol.
/// * `addend`   — Addend from the relocation entry (typically stored in
///                the low bits of the site before patching; for import
///                fixups the addend is already embedded in the code by
///                the compiler).
///
/// # Safety
///
/// `buf[site_off .. site_off + patch_size]` must be writable.
/// Callers must have already loaded the section into writable memory.
///
/// # Errors
///
/// Returns `E_RELOC_UNSUPPORTED` for any `kind` outside the x86_64 subset.
pub fn apply_import_reloc(
    buf: &mut [u8],
    site_off: usize,
    kind: u8,
    sym_addr: u64,
    addend: i64,
) -> Result<(), LoadError> {
    match kind {
        k if k == RelocKind::X86_64_64 as u8 => {
            // R_X86_64_64: S + A  (write 8 bytes, little-endian)
            if site_off + 8 > buf.len() {
                return Err(LoadError::RelocUnsupported);
            }
            let val = sym_addr.wrapping_add(addend as u64);
            buf[site_off..site_off + 8].copy_from_slice(&val.to_le_bytes());
            Ok(())
        }
        k if k == RelocKind::X86_64_PC32 as u8 || k == RelocKind::X86_64_PLT32 as u8 => {
            // R_X86_64_PC32 / R_X86_64_PLT32: S + A - P  (write 4 bytes, LE)
            if site_off + 4 > buf.len() {
                return Err(LoadError::RelocUnsupported);
            }
            let p = (buf.as_ptr() as u64).wrapping_add(site_off as u64);
            let val = (sym_addr as i64)
                .wrapping_add(addend)
                .wrapping_sub(p as i64);
            buf[site_off..site_off + 4].copy_from_slice(&(val as u32).to_le_bytes());
            Ok(())
        }
        _ => Err(LoadError::RelocUnsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // -----------------------------------------------------------------------
    // R_X86_64_64 tests
    // -----------------------------------------------------------------------

    #[test]
    fn abs64_writes_eight_bytes() {
        let mut buf = vec![0u8; 16];
        apply_import_reloc(&mut buf, 0, 1, 0x1234, 0).unwrap();
        let val = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(val, 0x1234);
    }

    #[test]
    fn abs64_with_addend() {
        let mut buf = vec![0u8; 16];
        apply_import_reloc(&mut buf, 0, 1, 0x1000, 0x234).unwrap();
        let val = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(val, 0x1234);
    }

    #[test]
    fn abs64_with_negative_addend() {
        let mut buf = vec![0u8; 16];
        apply_import_reloc(&mut buf, 0, 1, 0x1000, -0x100).unwrap();
        let val = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(val, 0x0f00);
    }

    #[test]
    fn abs64_at_nonzero_offset() {
        let mut buf = vec![0u8; 24];
        apply_import_reloc(&mut buf, 16, 1, 0x1000, 0x200).unwrap();
        let val = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        assert_eq!(val, 0x1200); // S + A = 0x1000 + 0x200
    }

    #[test]
    fn abs64_wrapping_arithmetic() {
        let mut buf = vec![0u8; 8];
        // sym_addr = u64::MAX, addend = 1 → wraps to 0
        apply_import_reloc(&mut buf, 0, 1, u64::MAX, 1).unwrap();
        let val = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(val, 0);
    }

    // -----------------------------------------------------------------------
    // R_X86_64_PC32 tests
    // -----------------------------------------------------------------------

    #[test]
    fn pc32_writes_four_bytes() {
        let mut buf = vec![0u8; 16];
        let p = buf.as_ptr() as u64 + 0; // site address in virtual memory
        let sym = 0x2000u64;
        let addend = 0i64;
        let expected = (sym as i64).wrapping_add(addend).wrapping_sub(p as i64);
        // PC32 stores a 32-bit signed value = (S + A - P) truncated to i32.
        let expected_32 = expected as i32;
        apply_import_reloc(&mut buf, 0, 2, sym, addend).unwrap();
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, expected_32);
    }

    #[test]
    fn pc32_sym_equals_site() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        let sym = p; // sym_addr == site_addr → delta = 0
        apply_import_reloc(&mut buf, 0, 2, sym, 0).unwrap();
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, 0);
    }

    #[test]
    fn pc32_forward_reference() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        let sym = p + 0x100; // target is ahead of site
        apply_import_reloc(&mut buf, 0, 2, sym, 0).unwrap();
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, 0x100);
    }

    #[test]
    fn pc32_backward_reference() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        let sym = p.wrapping_sub(0x100); // target is before site
        apply_import_reloc(&mut buf, 0, 2, sym, 0).unwrap();
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, -0x100i32);
    }

    #[test]
    fn pc32_with_addend() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        let sym = p + 0x100;
        apply_import_reloc(&mut buf, 0, 2, sym, -4).unwrap();
        // S + A - P = (p+0x100) + (-4) - p = 0xFC
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, 0xFC);
    }

    // -----------------------------------------------------------------------
    // R_X86_64_PLT32 — identical formula to PC32
    // -----------------------------------------------------------------------

    #[test]
    fn plt32_treated_as_pc32() {
        let mut buf = vec![0u8; 8];
        let p = buf.as_ptr() as u64;
        let sym = p + 0x100;
        apply_import_reloc(&mut buf, 0, 3, sym, 0).unwrap();
        let val = i32::from_le_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(val, 0x100);
    }

    // -----------------------------------------------------------------------
    // Rejection tests
    // -----------------------------------------------------------------------

    #[test]
    fn unsupported_kind_rejected() {
        let mut buf = vec![0u8; 16];
        // kind = 0 is reserved
        let err = apply_import_reloc(&mut buf, 0, 0, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }

    #[test]
    fn arm_reloc_rejected_on_x86_64() {
        let mut buf = vec![0u8; 16];
        // arm Abs32 = kind 4 — not valid on x86_64
        let err = apply_import_reloc(&mut buf, 0, 4, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }

    #[test]
    fn high_kind_value_rejected() {
        let mut buf = vec![0u8; 16];
        let err = apply_import_reloc(&mut buf, 0, 99, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }

    #[test]
    fn site_out_of_range_abs64() {
        let mut buf = vec![0u8; 4];
        let err = apply_import_reloc(&mut buf, 0, 1, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }

    #[test]
    fn site_out_of_range_pc32() {
        let mut buf = vec![0u8; 2];
        let err = apply_import_reloc(&mut buf, 0, 2, 0, 0).unwrap_err();
        assert_eq!(err, E_RELOC_UNSUPPORTED);
    }
}
