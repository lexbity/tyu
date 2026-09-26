//! Verifier-local target identity (PLAN-VERIFY-3 §Q3, slice P2.1).
//!
//! **Semantics is a function of `(TargetSpec, MemModel-instance)`, never a
//! constant** (§Q3): a statement proven on one target says nothing about
//! another. Every generated statement, verdict, and manifest record carries
//! a triple; the reference semantics ([`crate::interp`]) is parameterized by
//! this record and a [`crate::mem::MemModel`].
//!
//! This is a **deliberate duplicate** of the four target-spec fields the
//! backend owns — verification needs them but must not depend on the backend
//! crates (layering rule: `ir → verifier → (semantics, langc) → tyu`; ports
//! consume generated artifacts, never Rust crates). Parity with the backend's
//! values is asserted by `crates/tooling-tests/tests/target_parity.rs` (the
//! one place in the tree that may compare the two — it already depends on
//! both).

/// The four target-identity fields the reference semantics is relativized to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetSpec {
    /// Canonical target triple (`x86_64-unknown-none`, `armv7m-unknown-none`,
    /// `riscv32-unknown-none`, `x86_64-unknown-linux-gnu`).
    pub triple: &'static str,
    /// Data-stack slot width in bytes (abi-contract.md §2.1): 8 on x86_64,
    /// 4 on armv7-m / riscv32. Binds `usize`-class pointer arithmetic.
    pub slot_bytes: u8,
    /// Natural integer width in bits (64 / 32). Binds the `usize`/pointer
    /// value domain and the MMIO read width (§Q13).
    pub word_bits: u8,
    /// ABI architecture tag — MUST match `lmod::abi_hash::ARCH_TAG_*`
    /// (1 = SysV64/x86_64, 2 = AAPCS32/armv7-m, 3 = RISC-V ILP32).
    pub arch_tag: u8,
}

impl TargetSpec {
    /// A target-identity record. Used by the toolchain (langc) to lift the
    /// backend's resolved target into the verifier's boundary without a
    /// crate dependency.
    pub const fn new(
        triple: &'static str,
        slot_bytes: u8,
        word_bits: u8,
        arch_tag: u8,
    ) -> TargetSpec {
        TargetSpec {
            triple,
            slot_bytes,
            word_bits,
            arch_tag,
        }
    }

    /// The signed two's-complement domain of this target's natural word:
    /// `[-2^(word_bits-1), 2^(word_bits-1) - 1]` (see
    /// [`crate::interval::word_domain`]).
    pub const fn signed_domain(self) -> (i64, i64) {
        crate::interval::word_domain(self.word_bits)
    }

    /// The unsigned maximum of a `usize`/pointer of this target's word width:
    /// `2^word_bits − 1` (see [`crate::interval::usize_max`]).
    pub const fn usize_max(self) -> u64 {
        crate::interval::usize_max(self.word_bits)
    }

    /// The nondeterministic domain of an MMIO read of *this* target's word
    /// width (§Q13: a read yields a nondeterministic value **within the
    /// register's width**). `None` means the full i64 domain (`⊤`).
    pub const fn mmio_read_domain(self) -> Option<(i64, i64)> {
        if self.word_bits >= 64 {
            None
        } else {
            Some(self.signed_domain())
        }
    }

    /// The x86_64 reference target — the *default* every in-tree consumer
    /// uses (soundness harness, semantics lowering at x86_64, unit tests).
    pub const X86_64: TargetSpec = X86_64_UNKNOWN_NONE;
    pub const X86_64_UNKNOWN_LINUX_GNU: TargetSpec =
        TargetSpec::new("x86_64-unknown-linux-gnu", 8, 64, 1);
    pub const X86_64_UNKNOWN_NONE: TargetSpec = TargetSpec::new("x86_64-unknown-none", 8, 64, 1);
    pub const ARM_V7M_UNKNOWN_NONE: TargetSpec = TargetSpec::new("armv7m-unknown-none", 4, 32, 2);
    pub const RISCV32_UNKNOWN_NONE: TargetSpec = TargetSpec::new("riscv32-unknown-none", 4, 32, 3);
}

/// The four recognized target records, in parity with the backend's static
/// target table (constants — no heap).
pub const X86_64_UNKNOWN_LINUX_GNU: TargetSpec = TargetSpec::X86_64_UNKNOWN_LINUX_GNU;
pub const X86_64_UNKNOWN_NONE: TargetSpec = TargetSpec::X86_64_UNKNOWN_NONE;
pub const ARM_V7M_UNKNOWN_NONE: TargetSpec = TargetSpec::ARM_V7M_UNKNOWN_NONE;
pub const RISCV32_UNKNOWN_NONE: TargetSpec = TargetSpec::RISCV32_UNKNOWN_NONE;

/// Every recognized target, in a deterministic order.
pub const TARGETS: [TargetSpec; 4] = [
    X86_64_UNKNOWN_LINUX_GNU,
    X86_64_UNKNOWN_NONE,
    ARM_V7M_UNKNOWN_NONE,
    RISCV32_UNKNOWN_NONE,
];

/// Resolve a recognized target by its triple string.
pub fn for_triple(triple: &str) -> Option<TargetSpec> {
    TARGETS.iter().copied().find(|t| t.triple == triple)
}

/// The x86_64 reference target (module-level alias, same record as
/// `TargetSpec::X86_64`).
pub const X86_64: TargetSpec = TargetSpec::X86_64;
