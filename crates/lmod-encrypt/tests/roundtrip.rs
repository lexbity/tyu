//! Producer↔consumer contract tests (`lmod-encrypt` ↔ `loader-core`) have moved
//! to `tooling-tests/tests/contract_encryption.rs`.
//!
//! The tests that were here (`encrypt_roundtrip_fleet`, `tampered_ciphertext_rejected`,
//! `encrypt_then_sign_pipeline`) exercised circular oracles (re-implementing the AEAD
//! AAD inside the test).  They are replaced by C-CT-2, C-CT-3, C-CT-4, and the
//! rewritten signed-artifact test in `tooling-tests::contract_encryption` which use
//! the real loader as the oracle.
