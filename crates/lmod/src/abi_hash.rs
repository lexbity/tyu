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
///
/// v2 (2026-06-24): folded a target-identity discriminant (`arch_tag`) as the
/// first input so two targets with identical `slot_bytes`/`word_bits` but
/// different calling conventions (armv7-m vs riscv32, both 4/32) no longer
/// collide. Restores abi-contract §5 input #2 ("calling convention, data-stack
/// register convention"). See
/// devdocs/plans/platform-pack-contract-and-tooling-spec.md Phase 0a.
pub const ABI_HASH_VER: u64 = 2;

/// Stable target-identity tags folded into `abi_hash` (§5 input #2).
///
/// These MUST match `codegen_core::CallingConv::arch_tag()`. The values are a
/// permanent part of the wire contract — never renumber; only append.
pub const ARCH_TAG_X86_64: u8 = 1; // SysV64
pub const ARCH_TAG_ARM: u8 = 2; // AAPCS32 (armv7-m)
pub const ARCH_TAG_RISCV: u8 = 3; // RISC-V ILP32

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
///   - `arch_tag`: target-identity discriminant (`ARCH_TAG_*`), derived from the
///     calling convention. Distinguishes targets that share `slot_bytes`/
///     `word_bits` (armv7-m vs riscv32). MUST match
///     `codegen_core::CallingConv::arch_tag()`.
///   - `slot_bytes`: target-specific data-stack slot width (8 on x86_64).
///   - `word_bits`: target native integer width (64 on x86_64).
///   - `modinfo_ver`: `LangModInfo` struct version (must be `MODINFO_VER`).
///
/// Future versions will also fold in compiler version, type layout rules,
/// and trap-code assignments.  For v2 the hash is stable across rebuilds
/// because those quantities are themselves stable per toolchain release.
pub fn compute_abi_hash(arch_tag: u8, slot_bytes: u8, word_bits: u8, modinfo_ver: u16) -> u64 {
    let mut h: u64 = ABI_HASH_VER;

    // 1. Target identity (calling convention / data-stack register convention).
    h = fold_u64(h, arch_tag as u64);
    // 2. Target profile: slot_bytes
    h = fold_u64(h, slot_bytes as u64);
    // 3. Target profile: word_bits
    h = fold_u64(h, word_bits as u64);
    // 4. Runtime ABI version
    h = fold_u64(h, RUNTIME_ABI_VERSION);
    // 5. Effect/capability contract version
    h = fold_u64(h, modinfo_ver as u64);
    // 6. ABI hash recipe version (self-referential: guards against input
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
        let a = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 2);
        let b = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 2);
        assert_eq!(a, b);
    }

    #[test]
    fn runtime_abi_version_is_documented_value() {
        // The value must match abi-contract.md §4.4.3.
        // If the contract is revised, bump this constant and update the doc.
        assert_eq!(
            RUNTIME_ABI_VERSION, 1,
            "RUNTIME_ABI_VERSION must match abi-contract.md §4.4.3"
        );
    }

    #[test]
    fn abi_hash_changes_on_runtime_abi_version() {
        let base = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 2);
        // Simulate a bump of RUNTIME_ABI_VERSION by folding a different value.
        fn compute_with(arch: u8, slot: u8, bits: u8, ver: u16, rt: u64) -> u64 {
            let mut h: u64 = ABI_HASH_VER;
            h = fold_u64(h, arch as u64);
            h = fold_u64(h, slot as u64);
            h = fold_u64(h, bits as u64);
            h = fold_u64(h, rt);
            h = fold_u64(h, ver as u64);
            h = fold_u64(h, ABI_HASH_VER);
            h
        }
        let base2 = compute_with(ARCH_TAG_X86_64, 8, 64, 2, RUNTIME_ABI_VERSION);
        let bumped = compute_with(ARCH_TAG_X86_64, 8, 64, 2, RUNTIME_ABI_VERSION + 1);
        assert_eq!(base, base2, "compute_with must match compute_abi_hash");
        assert_ne!(
            base, bumped,
            "changing RUNTIME_ABI_VERSION must change abi_hash"
        );
    }

    #[test]
    fn abi_hash_changes_on_slot_bytes() {
        let base = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 2);
        let diff = compute_abi_hash(ARCH_TAG_X86_64, 4, 64, 2);
        assert_ne!(base, diff, "changing slot_bytes must change abi_hash");
    }

    #[test]
    fn abi_hash_changes_on_word_bits() {
        let base = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 2);
        let diff = compute_abi_hash(ARCH_TAG_X86_64, 8, 32, 2);
        assert_ne!(base, diff, "changing word_bits must change abi_hash");
    }

    #[test]
    fn abi_hash_changes_on_modinfo_ver() {
        let base = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 2);
        let diff = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 3);
        assert_ne!(base, diff, "changing modinfo_ver must change abi_hash");
    }

    #[test]
    fn abi_hash_non_zero() {
        let h = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 2);
        assert_ne!(h, 0, "abi_hash must not be zero");
    }

    /// AC-1 (Phase 0a): armv7-m and riscv32 share slot_bytes=4, word_bits=32.
    /// Before the arch_tag fix they hashed identically — an ARM module passed a
    /// RISC-V runtime's gate. The discriminant MUST separate them.
    #[test]
    fn abi_hash_arm_ne_riscv() {
        let arm = compute_abi_hash(ARCH_TAG_ARM, 4, 32, 3);
        let riscv = compute_abi_hash(ARCH_TAG_RISCV, 4, 32, 3);
        assert_ne!(
            arm, riscv,
            "armv7-m and riscv32 abi_hash MUST differ (cross-arch load must reject)"
        );
    }

    #[test]
    fn abi_hash_golden_x86_64_none() {
        // Golden for x86_64 (arch=1, slot_bytes=8, word_bits=64, MODINFO_VER=3).
        let h = compute_abi_hash(ARCH_TAG_X86_64, 8, 64, 3);
        assert_eq!(
            h, 0x41F0_5B8B_1ADA_B0ABu64,
            "abi_hash x86_64 golden must be stable"
        );
    }

    #[test]
    fn abi_hash_golden_armv7m_none() {
        // Golden for armv7-m (arch=2, slot_bytes=4, word_bits=32, MODINFO_VER=3).
        let h = compute_abi_hash(ARCH_TAG_ARM, 4, 32, 3);
        assert_eq!(
            h, 0x5D36_B0EF_E4B2_E904u64,
            "abi_hash armv7-m golden must be stable"
        );
    }

    #[test]
    fn abi_hash_golden_riscv32_none() {
        // Golden for riscv32 (arch=3, slot_bytes=4, word_bits=32, MODINFO_VER=3).
        let h = compute_abi_hash(ARCH_TAG_RISCV, 4, 32, 3);
        assert_eq!(
            h, 0xF6DD_34A3_E430_BD85u64,
            "abi_hash riscv32 golden must be stable"
        );
    }
}
