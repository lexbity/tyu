//! `abi_hash` — the module-level ABI compatibility guard.
//!
//! Canonical definition: abi-contract.md §5.
//! A single u64 embedded inside the signed region of every `.lmod` module
//! (module-format-and-loading.md §3). The loader rejects any module whose
//! `abi_hash` does not match `runtime.expected_abi_hash`.
//!
//! The hash covers every dimension that could cause silent misbehaviour
//! across a dynamic-load boundary: target profile data, runtime ABI version,
//! the effect/capability contract, and trap-code assignments.

/// Recipe version — bump when the input list or ordering changes.
pub const ABI_HASH_VER: u64 = 1;

/// Runtime ABI version — bump when the `__lang_*` symbol contract changes.
///
/// Canonical definition: abi-contract.md §4.4.3 (Runtime ABI bump discipline).
/// The contract sheet is at §4.4.1 (required exported symbols) and §4.4.2
/// (per-arch data-stack register / slot_bytes).
///
/// Bump this constant when:
/// - A symbol in the §4.4.1 table is added, removed, or renamed.
/// - The data-stack register (abi-contract §4.4.2) changes for an existing arch.
/// - The calling convention for `__lang_start` or `__lang_trap` changes.
///
/// A bump changes `abi_hash` for every module, so a mismatched runtime/module
/// pair is rejected at load (abi-contract §5 loader rule).
pub const RUNTIME_ABI_VERSION: u64 = 1;

// ---------------------------------------------------------------------------
// FNV-1a helpers
// ---------------------------------------------------------------------------

/// Feed one byte through the FNV-1a 64-bit hash.
#[inline]
fn fnv1a_byte(h: u64, b: u8) -> u64 {
    let h = h ^ (b as u64);
    h.wrapping_mul(1099511628211)
}

/// Feed a u64 (as 8 little-endian bytes) through the hash.
#[inline]
fn fold_u64(h: u64, v: u64) -> u64 {
    let le = v.to_le_bytes();
    let mut h = h;
    for &b in &le {
        h = fnv1a_byte(h, b);
    }
    h
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the module-level `abi_hash` from the canonical input list
/// (abi-contract §5, recipe version `ABI_HASH_VER`).
///
/// Parameters:
///   - `slot_bytes`: target-specific data-stack slot width (8 on x86_64).
///   - `word_bits`: target native integer width (64 on x86_64).
///   - `modinfo_ver`: `LangModInfo` struct version (must be `MODINFO_VER`).
///
/// Future versions will also fold in compiler version, type layout rules,
/// and trap-code assignments.  For v1 the hash is stable across rebuilds
/// because those quantities are themselves stable per toolchain release.
pub fn compute_abi_hash(slot_bytes: u8, word_bits: u8, modinfo_ver: u16) -> u64 {
    let mut h: u64 = ABI_HASH_VER;

    // 1. Target profile: slot_bytes
    h = fold_u64(h, slot_bytes as u64);
    // 2. Target profile: word_bits
    h = fold_u64(h, word_bits as u64);
    // 3. Runtime ABI version
    h = fold_u64(h, RUNTIME_ABI_VERSION);
    // 4. Effect/capability contract version
    h = fold_u64(h, modinfo_ver as u64);
    // 5. ABI hash recipe version (self-referential: guards against input
    //    list changes)
    h = fold_u64(h, ABI_HASH_VER);

    h
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_hash_is_deterministic() {
        let a = compute_abi_hash(8, 64, 2);
        let b = compute_abi_hash(8, 64, 2);
        assert_eq!(a, b);
    }

    #[test]
    fn runtime_abi_version_is_documented_value() {
        // The value must match abi-contract.md §4.4.3.
        // If the contract is revised, bump this constant and update the doc.
        assert_eq!(RUNTIME_ABI_VERSION, 1, "RUNTIME_ABI_VERSION must match abi-contract.md §4.4.3");
    }

    #[test]
    fn abi_hash_changes_on_runtime_abi_version() {
        let base = compute_abi_hash(8, 64, 2);
        // Simulate a bump of RUNTIME_ABI_VERSION by folding a different value.
        // (The actual constant is read inside compute_abi_hash, so we test
        //  indirectly by comparing: if someone changes the const, the hash
        //  must change.)
        fn compute_with(slot: u8, bits: u8, ver: u16, rt: u64) -> u64 {
            let mut h: u64 = ABI_HASH_VER;
            h = fold_u64(h, slot as u64);
            h = fold_u64(h, bits as u64);
            h = fold_u64(h, rt);
            h = fold_u64(h, ver as u64);
            h = fold_u64(h, ABI_HASH_VER);
            h
        }
        let base2 = compute_with(8, 64, 2, RUNTIME_ABI_VERSION);
        let bumped = compute_with(8, 64, 2, RUNTIME_ABI_VERSION + 1);
        assert_eq!(base, base2, "compute_with must match compute_abi_hash");
        assert_ne!(
            base, bumped,
            "changing RUNTIME_ABI_VERSION must change abi_hash"
        );
    }

    #[test]
    fn abi_hash_changes_on_slot_bytes() {
        let base = compute_abi_hash(8, 64, 2);
        let diff = compute_abi_hash(4, 64, 2);
        assert_ne!(
            base, diff,
            "changing slot_bytes must change abi_hash"
        );
    }

    #[test]
    fn abi_hash_changes_on_word_bits() {
        let base = compute_abi_hash(8, 64, 2);
        let diff = compute_abi_hash(8, 32, 2);
        assert_ne!(
            base, diff,
            "changing word_bits must change abi_hash"
        );
    }

    #[test]
    fn abi_hash_changes_on_modinfo_ver() {
        let base = compute_abi_hash(8, 64, 2);
        let diff = compute_abi_hash(8, 64, 3);
        assert_ne!(
            base, diff,
            "changing modinfo_ver must change abi_hash"
        );
    }

    #[test]
    fn abi_hash_non_zero() {
        let h = compute_abi_hash(8, 64, 2);
        assert_ne!(h, 0, "abi_hash must not be zero");
    }

    #[test]
    fn abi_hash_golden_x86_64_none() {
        // Golden value for x86_64-unknown-none (slot_bytes=8, word_bits=64,
        // MODINFO_VER=3).  If this changes, all previously-compiled modules
        // will be rejected by the loader — bump CODEGEN_REV and update any
        // dependent goldens.
        let h = compute_abi_hash(8, 64, 3);
        assert_eq!(h, 0x9100_D9DA_37DD_A42Au64,
            "abi_hash for slot_bytes=8, word_bits=64, modinfo_ver=3 must be stable");
    }

    #[test]
    fn abi_hash_golden_armv7m_none() {
        let h = compute_abi_hash(4, 32, 3);
        assert_eq!(h, 0x154D_A033_503B_0946u64,
            "abi_hash for slot_bytes=4, word_bits=32, modinfo_ver=3 must be stable");
    }
}
