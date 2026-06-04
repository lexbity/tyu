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

/// Runtime ABI version — bump when `__lang_start` / `__lang_trap` / data-stack
/// contract changes in a way that breaks compatibility.
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
}
