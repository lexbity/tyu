//! Target-identity parity (PLAN-VERIFY-3 P2.1): the verifier's local
//! [`verifier::target::TargetSpec`] must agree field-for-field with the
//! backend's `codegen_core::TargetSpec` for every recognized triple.
//!
//! The verifier deliberately does NOT depend on `codegen-core` (layering:
//! `ir → verifier → (semantics, langc) → tyu`; ports consume generated
//! artifacts, never Rust crates). The parity assertion therefore lives HERE,
//! in tooling-tests, which already depeds on both crates — the single place
//! in the tree that may compare them.

use codegen_core::Target;

/// Every recognized triple: the verifier triple string + the backend's
/// `(slot_bytes, word_bits, arch_tag)` MUST be identical.
#[test]
fn verifier_target_spec_matches_codegen() {
    let triples: &[&[u8]] = &[
        b"x86_64-unknown-linux-gnu",
        b"x86_64-unknown-none",
        b"armv7m-unknown-none",
        b"riscv32-unknown-none",
    ];
    for triple in triples {
        let backend = Target::parse(triple).expect("recognized backend triple");
        let spec = backend.spec();
        let triple_str = core::str::from_utf8(backend.triple()).expect("UTF-8 triple");
        let v = verifier::target::for_triple(triple_str)
            .unwrap_or_else(|| panic!("verifier must know triple {triple_str}"));
        assert_eq!(v.triple, triple_str, "triple string drift");
        assert_eq!(
            v.slot_bytes, spec.slot_bytes,
            "slot_bytes drift for {triple_str}"
        );
        assert_eq!(
            v.word_bits, spec.word_bits,
            "word_bits drift for {triple_str}"
        );
        assert_eq!(
            v.arch_tag,
            spec.calling_conv.arch_tag(),
            "arch_tag drift for {triple_str}"
        );
        // And the boundary ALWAYS agrees with the ABI tags (never renumber).
        assert!(
            v.arch_tag >= 1 && v.arch_tag <= 3,
            "arch_tag out of the closed set"
        );
    }
}

/// The verifier's static instances equal its own resolver (self-consistency).
#[test]
fn verifier_target_statics_are_consistent() {
    for t in verifier::target::TARGETS.iter() {
        assert_eq!(
            verifier::target::for_triple(t.triple),
            Some(*t),
            "resolver must return the static record for {}",
            t.triple
        );
    }
    assert_eq!(
        verifier::target::X86_64_UNKNOWN_NONE,
        verifier::target::X86_64
    );
}
