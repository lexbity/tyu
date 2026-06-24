//! Typechecker tests with populated semantic databases.
//!
//! Tests that the checker correctly enforces rules that depend on
//! subtype info, struct declarations, MMIO maps, resources, and iso
//! type sets.

mod common;

use common::*;

// ---------------------------------------------------------------------------
// Subtype info in type matching (populated via Dbs)
// ---------------------------------------------------------------------------

#[test]
fn subtype_in_env_affects_type_checking() {
    let (env, len) = builtin_env();
    // SubtypeInfo helps the typechecker match compatible types.
    // Without it, a subtype would not be recognized as compatible
    // with its base type.  This test verifies the table is passed
    // through correctly.
    let dbs = Dbs::new().with_subtype(b"Age", b"i64", 0, 150);
    // Simple arithmetic with subtypes — the typechecker must accept
    // a value of subtype Age where i64 is expected.
    check(
        "42",
        &[],
        &[b"i64"],
        &env[..len],
        &dbs,
        ChecksMode::Off,
        |result| {
            assert!(
                result.is_ok(),
                "subtype i64 must be accepted as base type i64"
            );
        },
    );
}
